// Text clipboard sync for an RDP tab. Rust moves the text between the two machines; this side only
// decides when to read the local clipboard (on focus and before a paste chord reaches the remote
// machine) and when to write what the remote side copied.

/** Largest text sent to the remote clipboard, in UTF-8 bytes (Rust drops anything bigger too). */
export const MAX_CLIPBOARD_BYTES = 1024 * 1024;

/** How long a read of the local clipboard may take; WebView2 can leave it pending on a prompt. */
export const READ_TIMEOUT_MS = 1500;

export interface ClipboardSyncOptions {
  read(): Promise<string>;
  write(text: string): Promise<void>;
  /** Hands text to the remote clipboard. */
  send(text: string): void;
  timeoutMs?: number;
  /** Called at most once, the first time the web view refuses a read or a write. */
  onBlocked?(): void;
}

const encoder = new TextEncoder();

export class ClipboardSync {
  #read: () => Promise<string>;
  #write: (text: string) => Promise<void>;
  #send: (text: string) => void;
  #timeoutMs: number;
  #onBlocked: (() => void) | undefined;
  /** The text both sides are known to hold; never sent back to where it came from. */
  #last: string | null = null;
  #blocked = false;

  constructor(options: ClipboardSyncOptions) {
    this.#read = options.read;
    this.#write = options.write;
    this.#send = options.send;
    this.#timeoutMs = options.timeoutMs ?? READ_TIMEOUT_MS;
    this.#onBlocked = options.onBlocked;
  }

  /** The remote side copied `text`: write it locally and remember it so it is not sent back. */
  async onRemote(text: string): Promise<void> {
    this.#last = text;
    try {
      await this.#write(text);
    } catch {
      this.#block();
    }
  }

  /**
   * Reads the local clipboard and sends it when it is text that changed, is not empty and fits the
   * cap. A read that fails or takes longer than the timeout counts as blocked; a late answer is
   * dropped, because it could belong to a later moment than the one that asked.
   */
  async syncLocal(): Promise<void> {
    let timer: ReturnType<typeof setTimeout> | undefined;
    const timeout = new Promise<null>((resolve) => {
      timer = setTimeout(() => resolve(null), this.#timeoutMs);
    });
    let text: string | null;
    try {
      text = await Promise.race([this.#read(), timeout]);
    } catch {
      text = null;
    } finally {
      clearTimeout(timer);
    }
    if (text === null) {
      this.#block();
      return;
    }
    if (text === "" || text === this.#last) return;
    if (encoder.encode(text).length > MAX_CLIPBOARD_BYTES) return;
    this.#last = text;
    this.#send(text);
  }

  #block(): void {
    if (this.#blocked) return;
    this.#blocked = true;
    this.#onBlocked?.();
  }
}

/** The keyboard paste chord: Ctrl+V (Cmd+V on a Mac) or Shift+Insert. */
export function isPasteChord(
  e: Pick<KeyboardEvent, "code" | "ctrlKey" | "metaKey" | "shiftKey" | "altKey">,
  mac: boolean,
): boolean {
  if (e.altKey) return false;
  if (e.code === "Insert") return e.shiftKey && !e.ctrlKey && !e.metaKey;
  return e.code === "KeyV" && (mac ? e.metaKey : e.ctrlKey);
}
