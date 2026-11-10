/** Wording for AI service connections. */
import type { ConnectionResult } from './api/bindings/ConnectionResult';
import type { CredentialStorage } from './api/bindings/CredentialStorage';
import type { OsKind } from './api/bindings/OsKind';
import type { ProviderKind } from './api/bindings/ProviderKind';
import type { ProviderLocation } from './api/bindings/ProviderLocation';

export const PRIVACY_NOTE =
  'Optional. Atlas works without AI. Testing asks a service for its models, or sends one short fixed request to check a model; nothing from your library is sent.';

/** Under a model list: listing isn't a capability claim. */
export const CAPABILITY_NOTE =
  "Listing a model doesn't show what it can do. Choose it under Models for Atlas to check it.";

export function locationLabel(location: ProviderLocation): string {
  switch (location) {
    case 'this_computer':
      return 'This computer';
    case 'another_computer':
      return 'Another computer';
    case 'internet':
      return 'Internet service';
  }
}

/** A request not sent because "Disable all external AI" is on. */
export const EXTERNAL_AI_OFF =
  'All external AI is turned off in Settings › Privacy, so nothing was sent to this internet service.';

export const INTERNET_NOTE =
  'An internet service: Atlas will ask before sending anything from your library to it.';

export interface Preset {
  id: string;
  label: string;
  kind: ProviderKind;
  baseUrl: string;
}

export const PRESETS: readonly Preset[] = [
  { id: 'ollama', label: 'Ollama', kind: 'ollama', baseUrl: 'http://localhost:11434' },
  {
    id: 'lmstudio',
    label: 'LM Studio',
    kind: 'openai_compatible',
    baseUrl: 'http://localhost:1234/v1',
  },
  { id: 'omlx', label: 'oMLX', kind: 'openai_compatible', baseUrl: 'http://localhost:8000/v1' },
  {
    id: 'openai',
    label: 'OpenAI',
    kind: 'openai_compatible',
    baseUrl: 'https://api.openai.com/v1',
  },
  { id: 'custom', label: 'Other OpenAI-compatible', kind: 'openai_compatible', baseUrl: '' },
];

export function kindLabel(kind: ProviderKind): string {
  return kind === 'ollama' ? 'Ollama' : 'OpenAI-compatible';
}

export function keychainName(os: OsKind | undefined): string {
  switch (os) {
    case 'macos':
      return 'Keychain';
    case 'windows':
      return 'Windows Credential Manager';
    default:
      return 'system keychain';
  }
}

export function storageLabel(storage: CredentialStorage, os: OsKind | undefined): string {
  switch (storage) {
    case 'keychain':
      return `Key saved in ${keychainName(os)}`;
    case 'session_only':
      return 'Key kept for this session only (re-enter after restarting Atlas)';
    case 'none':
      return 'No API key';
  }
}

/** True for plain-HTTP addresses that leave this computer. */
export function needsAcknowledgement(raw: string): boolean {
  try {
    const url = new URL(raw.trim());
    if (url.protocol !== 'http:') return false;
    const host = url.hostname.toLowerCase();
    return !(host === 'localhost' || host === '[::1]' || host === '::1' || /^127\./.test(host));
  } catch {
    return false;
  }
}

export function outcomeText(
  r: ConnectionResult,
  label: string,
  kind: ProviderKind,
  hadKey: boolean,
): string {
  const n = r.modelIds.length;
  switch (r.outcome) {
    case 'reachable':
      return `${label} answered and listed ${n.toLocaleString('en-US')} ${n === 1 ? 'model' : 'models'}.`;
    case 'no_models_listed':
      return kind === 'ollama'
        ? `Ollama${r.serverVersion ? ` ${r.serverVersion}` : ''} is running but has no models installed. Install one with Ollama, then test again.`
        : `${label} answered but listed no models.`;
    case 'authentication_failed':
      return hadKey
        ? 'The key was rejected. Check it and try again.'
        : `${label} needs an API key. Enter the key from ${label}'s settings.`;
    case 'network_error':
      return 'Nothing answered at this address. Is the service running?';
    case 'timeout':
      return 'The service took too long to answer.';
    case 'protocol_mismatch':
      return kind === 'ollama'
        ? 'This address answered, but not like Ollama. Ollama usually runs at http://localhost:11434.'
        : 'This address answered, but not like an OpenAI-compatible API. Check that the URL ends in /v1.';
    case 'insecure_url_rejected':
      return 'Unencrypted HTTP to another computer needs your confirmation first.';
    case 'external_ai_disabled':
      return EXTERNAL_AI_OFF;
  }
}

export function isSuccess(r: ConnectionResult): boolean {
  return r.outcome === 'reachable';
}
