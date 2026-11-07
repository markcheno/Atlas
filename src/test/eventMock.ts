/** Fake of `@tauri-apps/api/event`: tests push events with `emit`. */
import { vi } from 'vitest';

type Handler = (event: { event: string; payload: unknown }) => void;
const handlers = new Map<string, Set<Handler>>();

export const listen = vi.fn(async (event: string, handler: Handler) => {
  if (!handlers.has(event)) handlers.set(event, new Set());
  handlers.get(event)!.add(handler);
  return () => handlers.get(event)?.delete(handler);
});

export function emit(event: string, payload: unknown) {
  for (const handler of handlers.get(event) ?? []) handler({ event, payload });
}
