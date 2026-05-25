function hasTauriRuntime() {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

export type DialogMessageOptions = {
  title?: string;
  kind?: "info" | "warning" | "error";
  okLabel?: string;
  cancelLabel?: string;
};

export type EventPayload<T> = { payload: T };

export async function listen<T>(
  event: string,
  handler: (event: EventPayload<T>) => void,
): Promise<() => void> {
  if (!hasTauriRuntime()) {
    return () => {};
  }

  const mod = await import("@tauri-apps/api/event");
  return mod.listen<T>(event, handler);
}

export async function message(text: string, options?: DialogMessageOptions) {
  if (!hasTauriRuntime()) {
    window.alert(options?.title ? `${options.title}\n\n${text}` : text);
    return;
  }

  const mod = await import("@tauri-apps/plugin-dialog");
  await mod.message(text, options);
}

export async function confirm(text: string, options?: DialogMessageOptions) {
  if (!hasTauriRuntime()) {
    return window.confirm(options?.title ? `${options.title}\n\n${text}` : text);
  }

  const mod = await import("@tauri-apps/plugin-dialog");
  return mod.confirm(text, options);
}

export async function openDialog(options: Record<string, unknown>) {
  if (!hasTauriRuntime()) {
    throw new Error("file dialog requires Tauri runtime");
  }

  const mod = await import("@tauri-apps/plugin-dialog");
  return mod.open(options);
}

export function getCurrentWindow() {
  return {
    async isMaximized() {
      if (!hasTauriRuntime()) {
        return false;
      }

      const mod = await import("@tauri-apps/api/window");
      return mod.getCurrentWindow().isMaximized();
    },
    async toggleMaximize() {
      if (!hasTauriRuntime()) {
        return;
      }

      const mod = await import("@tauri-apps/api/window");
      await mod.getCurrentWindow().toggleMaximize();
    },
    async minimize() {
      if (!hasTauriRuntime()) {
        return;
      }

      const mod = await import("@tauri-apps/api/window");
      await mod.getCurrentWindow().minimize();
    },
    async close() {
      if (!hasTauriRuntime()) {
        return;
      }

      const mod = await import("@tauri-apps/api/window");
      await mod.getCurrentWindow().close();
    },
    async onResized(handler: () => void) {
      if (!hasTauriRuntime()) {
        return () => {};
      }

      const mod = await import("@tauri-apps/api/window");
      return mod.getCurrentWindow().onResized(handler);
    },
  };
}