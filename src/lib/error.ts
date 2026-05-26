import { writeLog } from "@/lib/api";

export function errorMessage(error: unknown) {
  return error instanceof Error ? error.message : String(error);
}

export function reportError(context: string, error: unknown) {
  const message = errorMessage(error);
  void writeLog("error", `${context}: ${message}`);
  return message;
}

export function reportDisplayedError(
  context: string,
  error: unknown,
  setError: (message: string) => void,
) {
  setError(reportError(context, error));
}