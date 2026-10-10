export type ToastKind = "info" | "success" | "error";
export interface Toast {
  id: number;
  kind: ToastKind;
  message: string;
}

class Toasts {
  items = $state<Toast[]>([]);
  private n = 0;

  push(kind: ToastKind, message: string, ms = kind === "error" ? 6000 : 3200) {
    const id = ++this.n;
    this.items.push({ id, kind, message });
    if (ms > 0) setTimeout(() => this.dismiss(id), ms);
    return id;
  }
  info = (m: string) => this.push("info", m);
  success = (m: string) => this.push("success", m);
  error = (e: unknown) => this.push("error", errorMessage(e));
  dismiss(id: number) {
    this.items = this.items.filter((t) => t.id !== id);
  }
}

export function errorMessage(e: unknown): string {
  if (e instanceof Error) return e.message;
  if (typeof e === "string") return e;
  if (e && typeof e === "object" && "message" in e) return String((e as { message: unknown }).message);
  return "Unexpected error";
}

export const toasts = new Toasts();
