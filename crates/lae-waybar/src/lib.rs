//! Waybar CFFI module — task label + workspace indicators.
//!
//! Each workspace button is keyed by its full Hyprland name (`auth-fix-2`, `3`, …).
//! The strip is rebuilt only on taskspace changes; within a taskspace, `workspacev2`
//! only flips the active CSS class on two existing labels.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use lae_core::{
    allowed_workspace_names, build_all_modules, fetch_occupied_names,
    hyprland_events::{
        is_full_refresh_event, is_monitor_focus_event, is_workspace_focus_event,
        parse_focusedmon_v2, parse_workspace_v2, HyprlandEventListener,
    },
    sync_from_workspace_name, trace_enabled, trace_event, visible_default_workspace_count,
    workspace_display_label, workspace_goto_name, Registry, SessionState, WaybarModuleJson,
    ACTIVE_WORKSPACE_ICON,
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
    /// Set during init on the Waybar/GTK thread; read only from `main_ctx.invoke` closures.
    static MAIN_RUNTIME: RefCell<Option<Rc<Runtime>>> = const { RefCell::new(None) };
}

struct Widgets {
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
    Full,
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
}

struct LaeBar {
    runtime: Rc<Runtime>,
    _listener: Option<HyprlandEventListener>,
    hypr_events: bool,
}

const WORKSPACE_BUTTON_CSS: &str = r#"
#lae-workspaces button {
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
#lae-workspaces button label {
  padding: 0;
  margin: 0;
}
#lae-workspaces button.empty {
  opacity: 0.5;
}
"#;

fn install_workspace_button_styles(workspace_box: &GtkBox) {
    let provider = CssProvider::new();
    if let Err(err) = provider.load_from_data(WORKSPACE_BUTTON_CSS.as_bytes()) {
        eprintln!("lae-waybar: failed to load workspace button CSS: {err}");
        return;
    }
    workspace_box
        .style_context()
        .add_provider(&provider, STYLE_PROVIDER_PRIORITY_APPLICATION);
}

const BUTTON_CLASSES: &[&str] = &["active", "empty", "idle", "global"];

const LABEL_CLASSES: &[&str] = &[
    "active", "empty", "global", "idle", "hidden", "task", "default",
];

fn normalize_hypr_workspace_name(name: &str) -> String {
    name.strip_prefix("name:")
        .unwrap_or(name)
        .trim()
        .to_string()
}

impl Runtime {
    fn merge_pending(slot: &mut Option<PendingRefresh>, refresh: PendingRefresh) {
        match (&*slot, &refresh) {
            (Some(PendingRefresh::Full), _) => {}
            (Some(PendingRefresh::Fast(_)), PendingRefresh::Full) => {
                *slot = Some(PendingRefresh::Full);
            }
            (_, PendingRefresh::Full) => *slot = Some(PendingRefresh::Full),
            (_, PendingRefresh::Fast(_)) => *slot = Some(refresh),
        }
    }

    fn queue_and_dispatch(
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
            MAIN_RUNTIME.with(|cell| {
                if let Some(runtime) = cell.borrow().as_ref() {
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
            Some(PendingRefresh::Full) => self.refresh_occupied(),
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

    fn taskspace_changed(&self, state: &SessionState, allowed: &[String]) -> bool {
        *self.taskspace_key.borrow() != state.taskspace_key() || *self.allowed.borrow() != allowed
    }

    fn apply_task_module(&self, state: &SessionState) {
        let modules = build_all_modules(state, false);
        if let Some(task) = modules.get("task") {
            apply_module(&self.widgets.task_label, task);
        } else {
            self.widgets.task_label.set_text("󰣇 default");
            self.widgets.task_label.set_visible(true);
        }
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
        global: bool,
        occupied: &HashSet<String>,
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
        if global {
            ctx.add_class("global");
        }
    }

    fn set_button_active(
        &self,
        name: &str,
        active: bool,
        global: bool,
        occupied: &HashSet<String>,
    ) {
        let Some(entry) = self.buttons.borrow().get(name).cloned() else {
            return;
        };
        Self::style_button(&entry, name, active, global, occupied);
    }

    fn flip_active(
        &self,
        old: &str,
        new: &str,
        global: bool,
        occupied: &HashSet<String>,
    ) {
        if old != new && self.buttons.borrow().contains_key(old) {
            self.set_button_active(old, false, global, occupied);
        }
        if self.buttons.borrow().contains_key(new) {
            self.set_button_active(new, true, global, occupied);
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
        let global = state.context_mode == lae_core::ContextMode::Global;
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
            Self::style_button(&entry, name, is_active, global, occupied);
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
        if *self.active_name.borrow() == workspace_name {
            return true;
        }
        if !self.buttons.borrow().contains_key(workspace_name) {
            return false;
        }

        let mut state_guard = self.state.borrow_mut();
        let Some(session) = state_guard.as_mut() else {
            return false;
        };

        let before_mode = session.context_mode;
        let before_task = session.current_task_id.clone();
        sync_from_workspace_name(session, workspace_name);
        let task_changed =
            before_mode != session.context_mode || before_task != session.current_task_id;

        let allowed = allowed_workspace_names(session);
        if task_changed || self.taskspace_changed(session, &allowed) {
            return false;
        }

        let new_active_rel = allowed
            .iter()
            .position(|n| n == workspace_name)
            .map(|i| (i + 1) as i32)
            .unwrap_or(1);
        let occupied = self.occupied.borrow();
        let new_visible = visible_default_workspace_count(
            session,
            &allowed,
            new_active_rel,
            &self.occupied_relative(&occupied),
        );
        if new_visible != *self.visible_count.borrow() {
            return false;
        }

        let old_active = self.active_name.borrow().clone();
        let global = session.context_mode == lae_core::ContextMode::Global;
        drop(occupied);
        drop(state_guard);
        self.flip_active(&old_active, workspace_name, global, &self.occupied.borrow());
        *self.active_name.borrow_mut() = workspace_name.to_string();
        trace_event("waybar", "flip", &format!("done old={old_active} new={workspace_name}"));
        true
    }

    fn repaint_fast(&self, workspace_name: &str) {
        let workspace_name = normalize_hypr_workspace_name(workspace_name);
        if workspace_name.is_empty() {
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

        let occupied = self.occupied.borrow().clone();
        let allowed = allowed_workspace_names(&state);
        let old_active = self.active_name.borrow().clone();
        let old_visible = *self.visible_count.borrow();
        let new_active_rel = allowed
            .iter()
            .position(|n| n == &workspace_name)
            .map(|i| (i + 1) as i32)
            .unwrap_or(1);
        let new_visible = visible_default_workspace_count(
            &state,
            &allowed,
            new_active_rel,
            &self.occupied_relative(&occupied),
        );
        let task_changed =
            before_mode != state.context_mode || before_task != state.current_task_id;
        let strip_changed = self.taskspace_changed(&state, &allowed);
        let visibility_changed = old_visible != new_visible;

        if task_changed {
            self.apply_task_module(&state);
        }

        if strip_changed || visibility_changed {
            self.rebuild_workspace_strip(&state, &allowed, &workspace_name, &occupied);
        } else if old_active != workspace_name {
            let global = state.context_mode == lae_core::ContextMode::Global;
            self.flip_active(&old_active, &workspace_name, global, &occupied);
        }

        self.store_snapshot(&state, &allowed, &occupied, &workspace_name);
    }

    fn repaint_full(&self) {
        if let Ok(registry) = Registry::with_defaults() {
            if let Ok(state) = registry.load_state() {
                let occupied = fetch_occupied_names(&state);
                let active_name = lae_core::hyprland::get_active_workspace()
                    .ok()
                    .flatten()
                    .map(|ws| ws.name)
                    .filter(|n| !n.is_empty());
                let mut synced = state;
                if let Some(ref name) = active_name {
                    sync_from_workspace_name(&mut synced, name);
                }
                let allowed = allowed_workspace_names(&synced);
                let active = active_name
                    .as_deref()
                    .map(normalize_hypr_workspace_name)
                    .filter(|n| !n.is_empty())
                    .filter(|n| allowed.iter().any(|a| a == n))
                    .or_else(|| allowed.first().cloned())
                    .unwrap_or_else(|| "1".into());

                self.apply_task_module(&synced);
                self.rebuild_workspace_strip(&synced, &allowed, &active, &occupied);
                self.store_snapshot(&synced, &allowed, &occupied, &active);
                return;
            }
        }
    }

    fn sync_active_workspace(&self) {
        if self.state.borrow().is_none() {
            return;
        }
        let Ok(Some(ws)) = lae_core::hyprland::get_active_workspace() else {
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
        if let Ok(registry) = Registry::with_defaults() {
            if let Ok(state) = registry.load_state() {
                let occupied = self.occupied.borrow().clone();
                let allowed = allowed_workspace_names(&state);
                let active = self.active_name.borrow().clone();

                self.apply_task_module(&state);

                if self.taskspace_changed(&state, &allowed) {
                    let active_ref = if allowed.iter().any(|n| n == &active) {
                        active.as_str()
                    } else {
                        allowed.first().map(String::as_str).unwrap_or("1")
                    };
                    self.rebuild_workspace_strip(&state, &allowed, active_ref, &occupied);
                    self.store_snapshot(&state, &allowed, &occupied, active_ref);
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
        let occupied = fetch_occupied_names(&state);
        let allowed = self.allowed.borrow().clone();
        let active = self.active_name.borrow().clone();
        let global = state.context_mode == lae_core::ContextMode::Global;

        for (name, entry) in self.buttons.borrow().iter() {
            let is_active = name == &active;
            Self::style_button(entry, name, is_active, global, &occupied);
        }

        self.store_snapshot(&state, &allowed, &occupied, &active);
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

impl LaeBar {
    fn start_hyprland_listener(
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
                            &pending,
                            &scheduled,
                            &main_ctx,
                            PendingRefresh::Fast(name),
                        );
                    }
                }
            } else if is_full_refresh_event(event) {
                Runtime::queue_and_dispatch(
                    &pending,
                    &scheduled,
                    &main_ctx,
                    PendingRefresh::Full,
                );
            }
        }))
    }
}

impl Module for LaeBar {
    type Config = Config;

    fn init(info: &InitInfo, _config: Config) -> Self {
        let container = info.get_root_widget();
        let root = GtkBox::new(Orientation::Horizontal, 0);
        root.set_widget_name("cffi-lae");
        root.set_valign(Align::Center);
        container.add(&root);

        let task_label = Label::new(None);
        task_label.set_widget_name("lae-task");
        task_label.set_margin_end(2);
        root.add(&task_label);

        let workspace_box = GtkBox::new(Orientation::Horizontal, 0);
        workspace_box.set_widget_name("lae-workspaces");
        install_workspace_button_styles(&workspace_box);
        root.add(&workspace_box);

        let pending = Arc::new(Mutex::new(None));
        let dispatch_scheduled = Arc::new(AtomicBool::new(false));
        let main_ctx = glib::MainContext::ref_thread_default();
        let runtime = Rc::new(Runtime {
            widgets: Rc::new(Widgets {
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
        });

        MAIN_RUNTIME.with(|cell| *cell.borrow_mut() = Some(runtime.clone()));

        let hypr_ids = Arc::new(Mutex::new(HashMap::new()));
        let listener = Self::start_hyprland_listener(
            pending,
            dispatch_scheduled,
            main_ctx,
            hypr_ids,
        );
        let hypr_events = listener.is_some();
        if !hypr_events {
            eprintln!("lae-waybar: Hyprland socket2 unavailable — fallback refresh only");
        }
        trace_event(
            "waybar",
            "init",
            &format!(
                "trace={} socket2={} pid={} path={}",
                trace_enabled(),
                hypr_events,
                std::process::id(),
                lae_core::socket2_path()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| lae_core::diagnose_socket2().reason)
            ),
        );

        runtime.repaint_full();

        Self {
            runtime,
            _listener: listener,
            hypr_events,
        }
    }

    fn update(&mut self) {
        if !self.hypr_events {
            self.runtime.sync_active_workspace();
            self.runtime.reload_from_daemon();
        }
    }

    fn refresh(&mut self, _signal: i32) {
        if !self.hypr_events {
            self.runtime.sync_active_workspace();
        }
        self.runtime.reload_from_daemon();
    }
}

waybar_module!(LaeBar);

#[derive(Debug, Default, Deserialize)]
struct Config {}
