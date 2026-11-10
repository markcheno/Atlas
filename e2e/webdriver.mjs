// A minimal W3C WebDriver client (https://www.w3.org/TR/webdriver2/) for
// the smoke test. It covers the handful of commands the smoke flow needs,
// so the repository doesn't carry a full browser-automation framework.

const ELEMENT = 'element-6066-11e4-a52e-4f735466cecf';

export class WebDriver {
  constructor(base = 'http://127.0.0.1:4444') {
    this.base = base;
    this.id = null;
  }

  async #call(method, path, body) {
    const res = await fetch(`${this.base}${path}`, {
      method,
      headers: body ? { 'content-type': 'application/json' } : undefined,
      body: body ? JSON.stringify(body) : undefined,
    });
    const json = await res.json().catch(() => ({}));
    if (!res.ok) {
      const v = json.value ?? {};
      throw new Error(`${method} ${path}: ${res.status} ${v.error ?? ''} ${v.message ?? ''}`);
    }
    return json.value;
  }

  #s(path) {
    return `/session/${this.id}${path}`;
  }

  async start(capabilities) {
    const value = await this.#call('POST', '/session', {
      capabilities: { alwaysMatch: capabilities },
    });
    this.id = value.sessionId;
    return value;
  }

  async quit() {
    if (!this.id) return;
    const id = this.id;
    this.id = null;
    await this.#call('DELETE', `/session/${id}`).catch(() => {});
  }

  /** Every element matching an XPath expression. */
  async all(xpath) {
    const found = await this.#call('POST', this.#s('/elements'), {
      using: 'xpath',
      value: xpath,
    });
    return found.map((e) => e[ELEMENT]);
  }

  async click(element) {
    await this.#call('POST', this.#s(`/element/${element}/click`), {});
  }

  async type(element, text) {
    await this.#call('POST', this.#s(`/element/${element}/value`), { text });
  }

  /** Empties a text field (Slice 2.7: the AI service form is prefilled). */
  async clear(element) {
    await this.#call('POST', this.#s(`/element/${element}/clear`), {});
  }

  /**
   * Chooses the option with this visible text in a `<select>`, firing the
   * `input` and `change` events a person's choice would. (Clicking an
   * `<option>` isn't handled alike by every native driver.)
   */
  async select(element, label) {
    const ok = await this.execute(
      `const s = arguments[0];
       const o = [...s.options].find((o) => o.textContent.trim() === arguments[1]);
       if (!o) return false;
       s.value = o.value;
       s.dispatchEvent(new Event('input', { bubbles: true }));
       s.dispatchEvent(new Event('change', { bubbles: true }));
       return true;`,
      [{ [ELEMENT]: element }, label],
    );
    if (!ok) throw new Error(`no option ${label}`);
  }

  async text(element) {
    return this.#call('GET', this.#s(`/element/${element}/text`));
  }

  async enabled(element) {
    return this.#call('GET', this.#s(`/element/${element}/enabled`));
  }

  async execute(script, args = []) {
    return this.#call('POST', this.#s('/execute/sync'), { script, args });
  }

  /** A base64 PNG of the window. */
  async screenshot() {
    return this.#call('GET', this.#s('/screenshot'));
  }

  /** Polls until an element matching `xpath` exists (and is enabled). */
  async waitFor(xpath, { timeout = 30_000, enabled = false } = {}) {
    const deadline = Date.now() + timeout;
    let last = null;
    while (Date.now() < deadline) {
      try {
        for (const el of await this.all(xpath)) {
          if (!enabled || (await this.enabled(el))) return el;
        }
      } catch (err) {
        last = err; // the page may be mid-render
      }
      await sleep(100);
    }
    throw new Error(`timed out waiting for ${xpath}${last ? ` (${last.message})` : ''}`);
  }

  /** Polls until `f()` returns something truthy. */
  async until(what, f, timeout = 30_000) {
    const deadline = Date.now() + timeout;
    while (Date.now() < deadline) {
      const v = await f().catch(() => null);
      if (v) return v;
      await sleep(200);
    }
    throw new Error(`timed out waiting for ${what}`);
  }
}

export const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

/** An XPath string literal for arbitrary text. */
export function lit(s) {
  if (!s.includes("'")) return `'${s}'`;
  if (!s.includes('"')) return `"${s}"`;
  return `concat('${s.split("'").join(`', "'", '`)}')`;
}

export const button = (label) => `//button[normalize-space(.)=${lit(label)}]`;
