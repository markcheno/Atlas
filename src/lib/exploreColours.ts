/**
 * The colours of Explore's areas (Slice 4.6.1): the eight hues of the
 * reference chart palette, each with a step for the light and the dark
 * surface. An area's hue index comes from Rust, chosen so that areas side
 * by side never share a hue nor a pair too close to tell apart.
 */
import type { ThemePreference } from './api/bindings/ThemePreference';

export const HUES = {
  light: ['#2a78d6', '#eb6834', '#1baf7a', '#eda100', '#e87ba4', '#008300', '#6250d6', '#e34948'],
  dark: ['#3987e5', '#d95926', '#199e70', '#c98500', '#d55181', '#008300', '#9085e9', '#e66767'],
} as const;

/** Whether the window shows its dark theme: chosen, or the system's. */
export function isDark(preference: ThemePreference): boolean {
  if (preference === 'dark') return true;
  if (preference === 'light') return false;
  return typeof window.matchMedia === 'function'
    ? window.matchMedia('(prefers-color-scheme: dark)').matches
    : false;
}

export function hueColour(hue: number, dark: boolean): string {
  const steps = dark ? HUES.dark : HUES.light;
  return steps[((hue % 8) + 8) % 8];
}
