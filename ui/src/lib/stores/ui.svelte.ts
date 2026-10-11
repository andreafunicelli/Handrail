import type { Connection } from "../api/types";

export type SettingsPage = "providers" | "harnesses" | "approvals" | "import" | "sync" | "appearance" | "credentials" | "vault" | "about";
export type RailView = "connections" | "snippets";

/** Transient UI state: overlays, drawers, palette. */
class Ui {
  rail = $state<RailView>("connections");
  settingsPage = $state<SettingsPage | null>(null);
  editing = $state<{ connection: Connection; isNew: boolean } | null>(null);
  paletteOpen = $state(false);
  /** Mobile/narrow layout: drawers instead of docked sidebars. */
  narrow = $state(false);
  sidebarDrawer = $state(false);
  aiDrawer = $state(false);
  saveWorkspaceOpen = $state(false);

  confirmReq = $state<{ title: string; body?: string; confirmLabel: string; danger: boolean; resolve: (ok: boolean) => void } | null>(null);

  /** Promise-based confirm dialog (window.confirm is not available in every webview). */
  ask(title: string, opts: { body?: string; confirmLabel: string; danger?: boolean }): Promise<boolean> {
    this.confirmReq?.resolve(false);
    return new Promise((resolve) => {
      this.confirmReq = { title, body: opts.body, confirmLabel: opts.confirmLabel, danger: opts.danger ?? false, resolve };
    });
  }
  answer(ok: boolean) {
    this.confirmReq?.resolve(ok);
    this.confirmReq = null;
  }

  openSettings(page: SettingsPage = "providers") {
    this.settingsPage = page;
    this.paletteOpen = false;
  }
  edit(connection: Connection, isNew = false) {
    this.editing = { connection: structuredClone($state.snapshot(connection) as Connection), isNew };
    this.sidebarDrawer = false;
  }
}

export const ui = new Ui();
