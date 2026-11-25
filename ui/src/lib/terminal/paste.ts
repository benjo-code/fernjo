// Filtering of pasted text before it reaches the terminal (M11 review #31). Text from the
// clipboard is not trusted: a hostile RDP server, a web page or any other program can put an
// embedded bracketed-paste end marker (`ESC[201~`) into it, which would end paste mode early so
// that the rest runs as typed input on the shell. As most current terminals strip ESC or the end
// marker from pasted text, this one drops the markers and every control character except tab,
// line feed and carriage return, so a multi-line paste keeps its lines.

const MARKERS = ["\u001b[200~", "\u001b[201~"];

/** C0 controls (ESC among them) except TAB, LF and CR; DEL; C1 controls (U+0080 to U+009F). */
function isDropped(code: number): boolean {
  if (code === 0x09 || code === 0x0a || code === 0x0d) return false;
  return code < 0x20 || (code >= 0x7f && code <= 0x9f);
}

/**
 * The text as it may be pasted: bracketed-paste markers and control characters (other than tab,
 * LF and CR) removed. A marker that removing another one forms again loses its ESC in the second
 * step, so no ESC is ever left.
 */
export function sanitizePaste(text: string): string {
  let out = text;
  for (const marker of MARKERS) out = out.split(marker).join("");
  let clean = "";
  for (let i = 0; i < out.length; i++) {
    const code = out.charCodeAt(i);
    if (isDropped(code)) continue;
    clean += out[i];
  }
  return clean;
}
