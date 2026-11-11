/**
 * Whether a key belongs to an input method still composing text (Japanese, Chinese, Korean,
 * Vietnamese telex and the like), so a key handler leaves it alone.
 *
 * Confirming a candidate sends Enter, and Space picks the next kanji, as ordinary keydowns:
 * WebView2 marks them `isComposing`, WebKitGTK sends them as keyCode 229. Without this check a
 * reminder was saved, a tag added or a field left with text the person hadn't confirmed yet.
 * Every handler that acts on Enter, Space, comma, Escape or arrows asks this first.
 */
export function composing(e: { nativeEvent?: { isComposing?: boolean }; isComposing?: boolean; keyCode?: number }): boolean {
  return Boolean(e.isComposing || e.nativeEvent?.isComposing || e.keyCode === 229);
}
