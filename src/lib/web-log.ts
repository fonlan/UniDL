import type { LogLevel } from "@/lib/api";
import { webRequest } from "@/lib/runtime";

export function writeWebLog(level: LogLevel, message: string): Promise<void> {
  return webRequest("/api/logs", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ level, message }),
  }).then(() => undefined);
}