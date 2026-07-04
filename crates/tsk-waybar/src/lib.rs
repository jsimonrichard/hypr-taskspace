//! Waybar CFFI module — task label + workspace indicators.
//!
//! Each workspace button is keyed by its full Hyprland name (`auth-fix-2`, `3`, …).
//! The strip is rebuilt only on taskspace changes; within a taskspace, `workspacev2`
//! only flips the active CSS class on two existing labels.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use tsk_core::{
    bar_active_workspace_name, bar_occupied_names, bar_workspace_names, build_all_modules,
    hyprland_events::{
        is_full_refresh_event, is_monitor_focus_event, is_workspace_focus_event,
        parse_focusedmon_v2, parse_workspace_v2, HyprlandEventListener,
    },
    is_global_workspace_name, read_state_rev, sync_from_workspace_name, trace_enabled, trace_event,
    visible_default_workspace_count, launch_task_tui, workspace_display_label,
    workspace_goto_name, ContextMode, Registry, SessionState, StateChangeKind, StateEventListener,
    WaybarModuleJson, ACTIVE_WORKSPACE_ICON,
};
use serde::Deserialize;
use waybar_cffi::{
    gtk::{
        glib, prelude::*, Align, Box as GtkBox, Button, CssProvider, Label, Orientation,
        ReliefStyle, STYLE_PROVIDER_PRIORITY_APPLICATION,
    },
    waybar_module, InitInfo, Module,
};

thread_local! {
    /// One entry per Waybar bar instance (multi-monitor); indexed by `Runtime::id`.
    static RUNTIMES: RefCell<Vec<Rc<Runtime>>> = RefCell::new(Vec::new());
}

struct Widgets {
    task_button: Button,
    task_label: Label,
    workspace_box: GtkBox,
}

#[derive(Clone)]
struct WorkspaceButton {
    button: Button,
    label: Label,
}

enum PendingRefresh {
    Fast(String),
    /// Taskspace / state.db change — rebuild strip when needed.
    Full,
    /// Window open/close — restyle occupied slots only.
    Occupied,
}

struct Runtime {
    widgets: Rc<Widgets>,
    /// Full Hyprland workspace name → clickable button.
    buttons: RefCell<HashMap<String, WorkspaceButton>>,
    state: RefCell<Option<SessionState>>,
    taskspace_key: RefCell<String>,
    allowed: RefCell<Vec<String>>,
    occupied: RefCell<HashSet<String>>,
    active_name: RefCell<String>,
    visible_count: RefCell<u32>,
    pending: Arc<Mutex<Option<PendingRefresh>>>,
    last_state_rev: Arc<AtomicU64>,
}

fn register_runtime(runtime: Rc<Runtime>) -> usize {
    RUNTIMES.with(|cell| {
        let mut runtimes = cell.borrow_mut();
        let id = runtimes.len();
        runtimes.push(runtime);
        id
    })
}

struct TskBar {
    runtime: Rc<Runtime>,
    _hypr_listener: Option<HyprlandEventListener>,
    _state_listener: Option<StateEventListener>,
    hypr_events: bool,
}

const BAR_BUTTON_CSS: &str = r#"
#tsk-task-btn {
  background: transparent;
  border: none;
  box-shadow: none;
  padding: 0 4px 0 6px;
  margin: 0 2px 0 0;
  min-height: 0;
  font-family: inherit;
  font-size: inherit;
  color: inherit;
}
#tsk-task-btn label {
  padding: 0;
  margin: 0;
}
#tsk-workspaces button {
  background: transparent;
  border: none;
  box-shadow: none;
  padding: 0 6px;
  margin: 0 1.5px;
  min-width: 9px;
  min-height: 0;
  font-family: inherit;
  font-size: inherit;
  color: inherit;
}
#tsk-workspaces button label {
  padding: 0;
  margin: 0;
}
#tsk-workspaces button.empty {
  opacity: 0.5;
}
#tsk-workspaces button.global {
  color: #a6e3a1;
}
"#;

fn install_bar_button_styles(root: &GtkBox) {
    let provider = CssProvider::new();
    if let Err(err) = provider.load_from_data(BAR_BUTTON_CSS.as_bytes()) {
        eprintln!("tsk-waybar: failed to load bar button CSS: {err}");
        return;
    }
    root.style_context()
        .add_provider(&provider, STYLE_PROVIDER_PRIORITY_APPLICATION);
}

const BUTTON_CLASSES: &[&str] = &["active", "empty", "idle", "global"];

const LABEL_CLASSES: &[&str] = &[
    "active", "empty", "idle", "hidden", "task", "default",
];

fn normalize_hypr_workspace_name(name: &str) -> String {
    name.strip_prefix("name:")
        .unwrap_or(name)
        .trim()
        .to_string()
}

impl Runtime {
    fn merge_pending(slot: &mut Option<PendingRefresh>, refresh: PendingRefresh) {
        use PendingRefresh::{Fast, Full, Occupied};
        match (slot.as_ref(), &refresh) {
            (_, Full) => {
                *slot = Some(Full);
            }
            (Some(Full), _) => {}
            (Some(Fast(_)), Fast(name)) | (None, Fast(name)) => {
                *slot = Some(Fast(name.clone()));
            }
            (Some(Occupied), Fast(name)) => {
                *slot = Some(Fast(name.clone()));
            }
            (Some(Fast(_)), Occupied) | (Some(Occupied), Occupied) => {}
            (None, Occupied) => {
                *slot = Some(Occupied);
            }
        }
    }

    fn queue_and_dispatch(
        runtime_id: usize,
        pending: &Arc<Mutex<Option<PendingRefresh>>>,
        scheduled: &Arc<AtomicBool>,
        main_ctx: &glib::MainContext,
        refresh: PendingRefresh,
    ) {
        {
            let mut slot = pending
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            Self::merge_pending(&mut slot, refresh);
        }
        if scheduled.swap(true, Ordering::AcqRel) {
            return;
        }
        let scheduled = scheduled.clone();
        let main_ctx = main_ctx.clone();
        main_ctx.invoke_with_priority(glib::Priority::HIGH, move || {
            scheduled.store(false, Ordering::Release);
            RUNTIMES.with(|cell| {
                if let Some(runtime) = cell.borrow().get(runtime_id) {
                    runtime.process_pending();
                }
            });
        });
    }

    fn process_pending(&self) {
        let next = self
            .pending
            .lock()
            .ok()
            .and_then(|mut g| g.take());
        match next {
            Some(PendingRefresh::Fast(name)) => self.repaint_fast(&name),
            Some(PendingRefresh::Full) => {
                if !self.reconcile_db_taskspace() {
                    self.repaint_full();
                }
            }
            Some(PendingRefresh::Occupied) => self.refresh_occupied(),
            None => {}
        }
    }

    fn occupied_relative(&self, occupied: &HashSet<String>) -> HashSet<i32> {
        self.allowed
            .borrow()
            .iter()
            .enumerate()
            .filter(|(_, n)| occupied.contains(*n))
            .map(|(i, _)| (i + 1) as i32)
            .collect()
    }

    fn taskspace_changed(&self, state: &SessionState, bar_names: &[String]) -> bool {
        *self.taskspace_key.borrow() != state.taskspace_key()
            || *self.allowed.borrow() != bar_names
    }

    /// Pick up taskspace changes written by the CLI (state.db) without a full Waybar restart.
    fn reconcile_db_taskspace(&self) -> bool {
        let Ok(registry) = Registry::with_defaults() else {
            return false;
        };
        let Ok(fresh) = registry.load_state() else {
            return false;
        };
        let fresh_bar = bar_workspace_names(&fresh);
        if self.taskspace_changed(&fresh, &fresh_bar) {
            self.repaint_full();
            return true;
        }
        false
    }

    fn poll_state_rev(&self) -> bool {
        let current = read_state_rev();
        let seen = self.last_state_rev.load(Ordering::Acquire);
        if current > seen {
            self.last_state_rev.store(current, Ordering::Release);
            trace_event("waybar", "state.rev", &format!("{seen} -> {current}"));
            self.repaint_full();
            return true;
        }
        false
    }

    fn apply_task_module(&self, state: &SessionState) {
        let modules = build_all_modules(state, false);
        if let Some(task) = modules.get("task") {
            apply_module(&self.widgets.task_label, task);
            let hidden = task
                .class
                .as_deref()
                .is_some_and(|c| c.contains("hidden"));
            self.widgets.task_button.set_visible(!hidden);
            let tooltip = task
                .tooltip
                .as_deref()
                .unwrap_or("Task manager (SUPER+Tab)");
            self.widgets.task_button.set_tooltip_text(Some(tooltip));
        } else {
            self.widgets.task_label.set_text("󰣇 default");
            self.widgets.task_label.set_visible(true);
            self.widgets.task_button.set_visible(true);
            self.widgets
                .task_button
                .set_tooltip_text(Some("Task manager (SUPER+Tab)"));
        }
    }

    fn on_task_clicked() {
        let _ = std::thread::spawn(|| {
            if let Err(err) = launch_task_tui() {
                eprintln!("tsk-waybar: task tui: {err}");
            }
        });
    }

    fn on_workspace_clicked(workspace_name: &str) {
        if let Ok(registry) = Registry::with_defaults() {
            if let Ok(mut state) = registry.load_state() {
                if workspace_goto_name(&mut state, workspace_name).is_some() {
                    let _ = registry.save_state(&state);
                }
            }
        }
    }

    fn style_button(
        entry: &WorkspaceButton,
        workspace_name: &str,
        active: bool,
        occupied: &HashSet<String>,
        state: Option<&SessionState>,
    ) {
        entry.button.set_relief(ReliefStyle::None);
        let text = if active {
            ACTIVE_WORKSPACE_ICON.to_string()
        } else {
            workspace_display_label(workspace_name)
        };
        entry.label.set_text(&text);
        entry.button.set_tooltip_text(Some(workspace_name));

        let ctx = entry.button.style_context();
        for class in BUTTON_CLASSES {
            ctx.remove_class(class);
        }
        if active {
            ctx.add_class("active");
        } else if !occupied.contains(workspace_name) {
            ctx.add_class("empty");
        } else {
            ctx.add_class("idle");
        }
        if state.is_some_and(|state| {
            state.context_mode == ContextMode::Task && is_global_workspace_name(workspace_name, state)
        }) {
            ctx.add_class("global");
        }
    }

    fn set_button_active(
        &self,
        name: &str,
        active: bool,
        occupied: &HashSet<String>,
    ) {
        let Some(entry) = self.buttons.borrow().get(name).cloned() else {
            return;
        };
        Self::style_button(
            &entry,
            name,
            active,
            occupied,
            self.state.borrow().as_ref(),
        );
    }

    fn flip_active(
        &self,
        old: &str,
        new: &str,
        occupied: &HashSet<String>,
    ) {
        if old != new && self.buttons.borrow().contains_key(old) {
            self.set_button_active(old, false, occupied);
        }
        if self.buttons.borrow().contains_key(new) {
            self.set_button_active(new, true, occupied);
        }
    }

    /// Rebuild the workspace strip — taskspace / allowed-list / visibility changes.
    fn rebuild_workspace_strip(
        &self,
        state: &SessionState,
        allowed: &[String],
        active: &str,
        occupied: &HashSet<String>,
    ) {
        let active_rel = allowed
            .iter()
            .position(|n| n == active)
            .map(|i| (i + 1) as i32)
            .unwrap_or(1);
        let visible = visible_default_workspace_count(
            state,
            allowed,
            active_rel,
            &self.occupied_relative(occupied),
        );

        for child in self.widgets.workspace_box.children() {
            self.widgets.workspace_box.remove(&child);
        }
        self.buttons.borrow_mut().clear();

        for (i, name) in allowed.iter().enumerate() {
            let slot = (i + 1) as u32;
            let is_active = name == active;
            if slot > visible && !is_active {
                continue;
            }

            let button = Button::new();
            let label = Label::new(None);
            button.add(&label);
            let entry = WorkspaceButton { button, label };
            Self::style_button(&entry, name, is_active, occupied, Some(state));
            let ws_name = name.clone();
            entry.button.connect_clicked(move |_| {
                Self::on_workspace_clicked(&ws_name);
            });
            self.widgets.workspace_box.add(&entry.button);
            self.buttons
                .borrow_mut()
                .insert(name.clone(), entry);
        }

        self.widgets.workspace_box.show_all();
        *self.visible_count.borrow_mut() = visible;
    }

    fn store_snapshot(
        &self,
        state: &SessionState,
        allowed: &[String],
        occupied: &HashSet<String>,
        active: &str,
    ) {
        let active_rel = allowed
            .iter()
            .position(|n| n == active)
            .map(|i| (i + 1) as i32)
            .unwrap_or(1);
        let visible = visible_default_workspace_count(
            state,
            allowed,
            active_rel,
            &self.occupied_relative(occupied),
        );
        *self.state.borrow_mut() = Some(state.clone());
        *self.taskspace_key.borrow_mut() = state.taskspace_key();
        *self.allowed.borrow_mut() = allowed.to_vec();
        *self.occupied.borrow_mut() = occupied.clone();
        *self.active_name.borrow_mut() = active.to_string();
        *self.visible_count.borrow_mut() = visible;
    }

    fn try_flip_only(&self, workspace_name: &str) -> bool {
        let mut state_guard = self.state.borrow_mut();
        let Some(session) = state_guard.as_mut() else {
            return false;
        };

        let before_mode = session.context_mode;
        let before_task = session.current_task_id.clone();
        sync_from_workspace_name(session, workspace_name);
        let task_changed =
            before_mode != session.context_mode || before_task != session.current_task_id;
        let context_changed = before_mode != session.context_mode;

        let bar = bar_workspace_names(session);
        if task_changed || context_changed || self.taskspace_changed(session, &bar) {
            return false;
        }

        let bar_active = bar_active_workspace_name(workspace_name, session, &bar);
        if *self.active_name.borrow() == bar_active {
            return true;
        }
        if !self.buttons.borrow().contains_key(&bar_active) {
            return false;
        }

        let new_active_rel = bar
            .iter()
            .position(|n| n == &bar_active)
            .map(|i| (i + 1) as i32)
            .unwrap_or(1);
        let occupied = self.occupied.borrow();
        let new_visible = visible_default_workspace_count(
            session,
            &bar,
            new_active_rel,
            &self.occupied_relative(&occupied),
        );
        let old_visible = *self.visible_count.borrow();
        let old_active = self.active_name.borrow().clone();
        if new_visible != old_visible {
            drop(occupied);
            self.rebuild_workspace_strip(session, &bar, &bar_active, &self.occupied.borrow());
            *self.active_name.borrow_mut() = bar_active.clone();
            trace_event(
                "waybar",
                "rebuild",
                &format!(
                    "visible {old_visible}->{new_visible} active={bar_active} hypr={workspace_name}"
                ),
            );
            return true;
        }

        drop(occupied);
        drop(state_guard);
        self.flip_active(&old_active, &bar_active, &self.occupied.borrow());
        trace_event(
            "waybar",
            "flip",
            &format!("done old={old_active} new={bar_active} hypr={workspace_name}"),
        );
        *self.active_name.borrow_mut() = bar_active;
        true
    }

    fn repaint_fast(&self, workspace_name: &str) {
        let workspace_name = normalize_hypr_workspace_name(workspace_name);
        if workspace_name.is_empty() {
            return;
        }

        if self.reconcile_db_taskspace() {
            return;
        }

        if self.try_flip_only(&workspace_name) {
            return;
        }

        let mut state = match self.state.borrow_mut().take() {
            Some(s) => s,
            None => {
                self.repaint_full();
                return;
            }
        };

        let before_mode = state.context_mode;
        let before_task = state.current_task_id.clone();
        sync_from_workspace_name(&mut state, &workspace_name);

        let context_changed = before_mode != state.context_mode;
        let task_changed =
            context_changed || before_task != state.current_task_id;
        let bar = bar_workspace_names(&state);
        let strip_changed = self.taskspace_changed(&state, &bar);
        let occupied = if strip_changed || task_changed || context_changed {
            bar_occupied_names(&state, &bar)
        } else {
            self.occupied.borrow().clone()
        };
        let bar_active = bar_active_workspace_name(&workspace_name, &state, &bar);
        let old_active = self.active_name.borrow().clone();
        let old_visible = *self.visible_count.borrow();
        let new_active_rel = bar
            .iter()
            .position(|n| n == &bar_active)
            .map(|i| (i + 1) as i32)
            .unwrap_or(1);
        let new_visible = visible_default_workspace_count(
            &state,
            &bar,
            new_active_rel,
            &self.occupied_relative(&occupied),
        );
        let visibility_changed = old_visible != new_visible;

        if task_changed || context_changed {
            self.apply_task_module(&state);
        }

        if strip_changed || visibility_changed {
            self.rebuild_workspace_strip(&state, &bar, &bar_active, &occupied);
        } else if old_active != bar_active {
            self.flip_active(&old_active, &bar_active, &occupied);
        }

        self.store_snapshot(&state, &bar, &occupied, &bar_active);
    }

    fn repaint_full(&self) {
        if let Ok(registry) = Registry::with_defaults() {
            if let Ok(state) = registry.load_state() {
                let active_name = tsk_core::hyprland::get_active_workspace()
                    .ok()
                    .flatten()
                    .map(|ws| ws.name)
                    .filter(|n| !n.is_empty());
                let mut synced = state;
                if let Some(ref name) = active_name {
                    sync_from_workspace_name(&mut synced, name);
                }
                let bar = bar_workspace_names(&synced);
                let occupied = bar_occupied_names(&synced, &bar);
                let active = active_name
                    .as_deref()
                    .map(normalize_hypr_workspace_name)
                    .filter(|n| !n.is_empty())
                    .map(|n| bar_active_workspace_name(&n, &synced, &bar))
                    .filter(|n| bar.iter().any(|b| b == n))
                    .unwrap_or_else(|| bar.first().cloned().unwrap_or_else(|| "1".into()));

                self.apply_task_module(&synced);
                self.rebuild_workspace_strip(&synced, &bar, &active, &occupied);
                self.store_snapshot(&synced, &bar, &occupied, &active);
                return;
            }
        }
    }

    fn sync_active_workspace(&self) {
        if self.state.borrow().is_none() {
            return;
        }
        let Ok(Some(ws)) = tsk_core::hyprland::get_active_workspace() else {
            return;
        };
        let name = normalize_hypr_workspace_name(&ws.name);
        if name.is_empty() || name == *self.active_name.borrow() {
            return;
        }
        trace_event("waybar", "sync", &format!("hyprctl active={name}"));
        self.repaint_fast(&name);
    }

    fn reload_from_daemon(&self) {
        if self.reconcile_db_taskspace() {
            return;
        }
        if let Ok(registry) = Registry::with_defaults() {
            if let Ok(state) = registry.load_state() {
                let bar = bar_workspace_names(&state);
                let active = self.active_name.borrow().clone();

                self.apply_task_module(&state);

                if self.taskspace_changed(&state, &bar) {
                    let occupied = bar_occupied_names(&state, &bar);
                    let active_ref = if bar.iter().any(|n| n == &active) {
                        active.as_str()
                    } else {
                        bar.first().map(String::as_str).unwrap_or("1")
                    };
                    self.rebuild_workspace_strip(&state, &bar, active_ref, &occupied);
                    self.store_snapshot(&state, &bar, &occupied, active_ref);
                } else {
                    *self.state.borrow_mut() = Some(state.clone());
                    *self.taskspace_key.borrow_mut() = state.taskspace_key();
                }
            }
        }
    }

    fn refresh_occupied(&self) {
        let Some(state) = self.state.borrow().clone() else {
            self.repaint_full();
            return;
        };
        let bar = bar_workspace_names(&state);
        let occupied = bar_occupied_names(&state, &bar);
        let active = self.active_name.borrow().clone();
        for (name, entry) in self.buttons.borrow().iter() {
            let is_active = name == &active;
            Self::style_button(
                entry,
                name,
                is_active,
                &occupied,
                self.state.borrow().as_ref(),
            );
        }

        self.store_snapshot(&state, &bar, &occupied, &active);
    }
}

fn apply_module(label: &Label, module: &WaybarModuleJson) {
    label.set_text(&module.text);
    if let Some(tooltip) = &module.tooltip {
        label.set_tooltip_text(Some(tooltip));
    } else {
        label.set_tooltip_text(None::<&str>);
    }
    label.set_visible(!module.class.as_deref().is_some_and(|c| c.contains("hidden")));

    let ctx = label.style_context();
    for class in LABEL_CLASSES {
        ctx.remove_class(class);
    }
    if let Some(class) = &module.class {
        for token in class.split_whitespace() {
            ctx.add_class(token);
        }
    }
}

impl TskBar {
    fn start_state_event_listener(
        runtime_id: usize,
        pending: Arc<Mutex<Option<PendingRefresh>>>,
        scheduled: Arc<AtomicBool>,
        main_ctx: glib::MainContext,
        last_state_rev: Arc<AtomicU64>,
    ) -> Option<StateEventListener> {
        StateEventListener::start(Arc::new(move |kind, rev, workspace| {
            last_state_rev.store(rev, Ordering::Release);
            trace_event("waybar", "state.event", &format!("{kind:?} rev={rev}"));
            let refresh = match kind {
                StateChangeKind::Taskspace | StateChangeKind::Full => PendingRefresh::Full,
                StateChangeKind::Workspace => workspace
                    .map(PendingRefresh::Fast)
                    .unwrap_or(PendingRefresh::Full),
            };
            Runtime::queue_and_dispatch(
                runtime_id,
                &pending,
                &scheduled,
                &main_ctx,
                refresh,
            );
        }))
    }

    fn start_hyprland_listener(
        runtime_id: usize,
        pending: Arc<Mutex<Option<PendingRefresh>>>,
        scheduled: Arc<AtomicBool>,
        main_ctx: glib::MainContext,
        hypr_ids: Arc<Mutex<HashMap<i32, String>>>,
    ) -> Option<HyprlandEventListener> {
        HyprlandEventListener::start(Arc::new(move |event, payload| {
            if is_workspace_focus_event(event) {
                if let Some((id, name)) = parse_workspace_v2(payload) {
                    let name = normalize_hypr_workspace_name(&name);
                    if name.is_empty() {
                        return;
                    }
                    trace_event(
                        "waybar",
                        "socket",
                        &format!("workspacev2 id={id} name={name}"),
                    );
                    hypr_ids.lock().ok().map(|mut g| g.insert(id, name.clone()));
                    Runtime::queue_and_dispatch(
                        runtime_id,
                        &pending,
                        &scheduled,
                        &main_ctx,
                        PendingRefresh::Fast(name),
                    );
                }
            } else if is_monitor_focus_event(event) {
                if let Some(id) = parse_focusedmon_v2(payload) {
                    let name = hypr_ids
                        .lock()
                        .ok()
                        .and_then(|g| g.get(&id).cloned())
                        .or_else(|| ((1..=10).contains(&id)).then(|| id.to_string()));
                    if let Some(name) = name {
                        Runtime::queue_and_dispatch(
                            runtime_id,
                            &pending,
                            &scheduled,
                            &main_ctx,
                            PendingRefresh::Fast(name),
                        );
                    }
                }
            } else if is_full_refresh_event(event) {
                Runtime::queue_and_dispatch(
                    runtime_id,
                    &pending,
                    &scheduled,
                    &main_ctx,
                    PendingRefresh::Occupied,
                );
            }
        }))
    }
}

impl Module for TskBar {
    type Config = Config;

    fn init(info: &InitInfo, _config: Config) -> Self {
        let container = info.get_root_widget();
        let root = GtkBox::new(Orientation::Horizontal, 0);
        root.set_widget_name("cffi-tsk");
        root.set_valign(Align::Center);
        container.add(&root);

        let task_button = Button::new();
        task_button.set_widget_name("tsk-task-btn");
        task_button.set_relief(ReliefStyle::None);
        let task_label = Label::new(None);
        task_label.set_widget_name("tsk-task");
        task_button.add(&task_label);
        task_button.connect_clicked(|_| Runtime::on_task_clicked());
        root.add(&task_button);

        let workspace_box = GtkBox::new(Orientation::Horizontal, 0);
        workspace_box.set_widget_name("tsk-workspaces");
        install_bar_button_styles(&root);
        root.add(&workspace_box);

        let pending = Arc::new(Mutex::new(None));
        let dispatch_scheduled = Arc::new(AtomicBool::new(false));
        let main_ctx = glib::MainContext::ref_thread_default();
        let last_state_rev = Arc::new(AtomicU64::new(read_state_rev()));
        let runtime = Rc::new(Runtime {
            widgets: Rc::new(Widgets {
                task_button,
                task_label,
                workspace_box,
            }),
            buttons: RefCell::new(HashMap::new()),
            state: RefCell::new(None),
            taskspace_key: RefCell::new(String::new()),
            allowed: RefCell::new(Vec::new()),
            occupied: RefCell::new(HashSet::new()),
            active_name: RefCell::new(String::new()),
            visible_count: RefCell::new(5),
            pending: pending.clone(),
            last_state_rev: last_state_rev.clone(),
        });
        let runtime_id = register_runtime(runtime.clone());

        let state_listener = Self::start_state_event_listener(
            runtime_id,
            pending.clone(),
            dispatch_scheduled.clone(),
            main_ctx.clone(),
            last_state_rev,
        );

        let hypr_ids = Arc::new(Mutex::new(HashMap::new()));
        let hypr_listener = Self::start_hyprland_listener(
            runtime_id,
            pending,
            dispatch_scheduled,
            main_ctx,
            hypr_ids,
        );
        let hypr_events = hypr_listener.is_some();
        if !hypr_events {
            eprintln!("tsk-waybar: Hyprland socket2 unavailable — fallback refresh only");
        }
        if state_listener.is_none() {
            eprintln!("tsk-waybar: state-events socket unavailable — using rev polling");
        }
        trace_event(
            "waybar",
            "init",
            &format!(
                "trace={} socket2={} state_events={} pid={} path={}",
                trace_enabled(),
                hypr_events,
                state_listener.is_some(),
                std::process::id(),
                tsk_core::socket2_path()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| tsk_core::diagnose_socket2().reason)
            ),
        );

        runtime.repaint_full();

        Self {
            runtime,
            _hypr_listener: hypr_listener,
            _state_listener: state_listener,
            hypr_events,
        }
    }

    fn update(&mut self) {
        if self.runtime.poll_state_rev() {
            return;
        }
        if self.runtime.reconcile_db_taskspace() {
            return;
        }
        self.runtime.sync_active_workspace();
        if !self.hypr_events {
            self.runtime.reload_from_daemon();
        }
    }

    fn refresh(&mut self, _signal: i32) {
        if self.runtime.poll_state_rev() {
            return;
        }
        if self.runtime.reconcile_db_taskspace() {
            return;
        }
        self.runtime.sync_active_workspace();
        if !self.hypr_events {
            self.runtime.reload_from_daemon();
        }
    }
}

waybar_module!(TskBar);

#[derive(Debug, Default, Deserialize)]
struct Config {}
