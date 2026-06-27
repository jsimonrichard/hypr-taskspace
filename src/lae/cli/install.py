"""Install / uninstall subcommands."""

from __future__ import annotations

import typer

app = typer.Typer(help="Install lae integration with Hyprland and Waybar.")


def _echo_reload(actions: list[str] | None) -> None:
    if not actions:
        return
    typer.echo(f"Applied: {', '.join(actions)}.")


def _echo_hypr_install(plan) -> None:
    typer.echo("Installed Hyprland integration.")
    typer.echo(f"  bindings → {plan.templates[0][1].parent}")
    if plan.elephant_menu:
        typer.echo(f"  walker menu → {plan.elephant_symlink}")
    typer.echo(f"  backup → {plan.backup_dir}")
    typer.echo("Keybinds: SUPER+1..9, SUPER+0 (workspace 10), SUPER+Tab (task menu)")
    _echo_reload(plan.reload_actions)


def _echo_waybar_install(plan) -> None:
    typer.echo("Installed Waybar integration.")
    typer.echo(f"  config → {plan.config_path}")
    typer.echo(f"  backup → {plan.backup_dir}")
    _echo_reload(plan.reload_actions)


@app.command("all")
def install_all(
    dry_run: bool = typer.Option(False, "--dry-run", help="Show planned changes only"),
) -> None:
    """Install Hyprland + Waybar integrations and reload each component."""
    from lae.install.hypr import install_hypr as do_install_hypr
    from lae.install.hypr import plan_install as plan_hypr
    from lae.install.waybar import install_waybar, plan_install as plan_waybar

    if dry_run:
        hypr = plan_hypr()
        waybar = plan_waybar()
        typer.echo("Dry run — planned actions:")
        typer.echo("")
        typer.echo("Hyprland:")
        for src, dest in hypr.templates:
            typer.echo(f"  copy {src} → {dest}")
        if hypr.elephant_menu:
            src, dest = hypr.elephant_menu
            typer.echo(f"  copy {src} → {dest}")
            typer.echo(f"  symlink {hypr.elephant_symlink} → {dest}")
        typer.echo(f"  backup dir: {hypr.backup_dir}")
        typer.echo(f"  append to {hypr.config_path}:")
        typer.echo(f"    {hypr.source_line}")
        typer.echo("")
        typer.echo("Waybar:")
        for src, dest in waybar.templates:
            typer.echo(f"  copy {src} → {dest}")
        typer.echo(f"  patch {waybar.config_path}")
        if waybar.modules_left_before is not None:
            typer.echo(f"  replace hyprland/workspaces → {waybar.modules_left_before}")
        return

    hypr_plan = do_install_hypr()
    _echo_hypr_install(hypr_plan)
    typer.echo("")
    waybar_plan = install_waybar()
    _echo_waybar_install(waybar_plan)


@app.command("hypr")
def install_hypr(
    dry_run: bool = typer.Option(False, "--dry-run", help="Show planned changes only"),
) -> None:
    """Install lae Hyprland keybinds and window rules."""
    from lae.install.hypr import install_hypr as do_install
    from lae.install.hypr import plan_install

    if dry_run:
        plan = plan_install()
        typer.echo("Dry run — planned actions:")
        for src, dest in plan.templates:
            typer.echo(f"  copy {src} → {dest}")
        if plan.elephant_menu:
            src, dest = plan.elephant_menu
            typer.echo(f"  copy {src} → {dest}")
            typer.echo(f"  symlink {plan.elephant_symlink} → {dest}")
        typer.echo(f"  backup dir: {plan.backup_dir}")
        typer.echo(f"  append to {plan.config_path}:")
        typer.echo(f"    {plan.source_line}")
        return

    plan = do_install()
    _echo_hypr_install(plan)


@app.command("status")
def install_status_cmd() -> None:
    """Show install state."""
    from lae.install.hypr import install_status as hypr_status
    from lae.install.waybar import install_status as waybar_status

    h = hypr_status()
    if h["installed"]:
        typer.echo("Hyprland integration: installed")
        typer.echo(f"  config: {h['config_path']}")
        typer.echo(f"  bindings: {h['bindings_path']}")
        typer.echo(f"  source line present: {h['source_line_present']}")
    else:
        typer.echo("Hyprland integration: not installed")

    w = waybar_status()
    if w["installed"] or w["lae_modules_present"]:
        typer.echo("Waybar integration: installed")
        typer.echo(f"  config: {w['config_path']}")
        typer.echo(f"  lae modules present: {w['lae_modules_present']}")
    else:
        typer.echo("Waybar integration: not installed")


@app.command("waybar")
def install_waybar_cmd(
    dry_run: bool = typer.Option(False, "--dry-run"),
) -> None:
    """Replace hyprland/workspaces with lae task + desktop indicators."""
    from lae.install.waybar import install_waybar, plan_install

    if dry_run:
        plan = plan_install()
        typer.echo("Dry run — planned Waybar changes:")
        for src, dest in plan.templates:
            typer.echo(f"  copy {src} → {dest}")
        typer.echo(f"  patch {plan.config_path}")
        typer.echo(f"  replace hyprland/workspaces → {plan.modules_left_before}")
        return

    plan = install_waybar()
    _echo_waybar_install(plan)
