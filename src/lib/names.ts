/**
 * A person's name in the order its own language writes it. Written wholly in Chinese, Japanese
 * or Korean script, it is family name first with nothing between (王小明, 宮崎駿, 김민준), as
 * Apple's Contacts shows it (Unicode CLDR person names; Korean spelling rule 48 writes the two
 * together too). Anything else is given name first, a space between. Only how it is shown:
 * the card itself is never changed.
 */
const CJK = /^[\p{Script=Han}\p{Script=Hiragana}\p{Script=Katakana}\p{Script=Hangul}ー々〆・]+$/u;

/** Whether the name is written family name first, nothing between: wholly CJK script. */
export function familyFirst(given?: string | null, middle?: string | null, family?: string | null): boolean {
  const named = [given, middle, family].map((p) => p?.trim() ?? "").filter(Boolean);
  return named.length > 0 && named.every((p) => CJK.test(p));
}

export function personName(given?: string | null, middle?: string | null, family?: string | null): string {
  const parts = [given, middle, family].map((p) => p?.trim() ?? "");
  const named = parts.filter(Boolean);
  if (familyFirst(given, middle, family)) {
    const [g, m, f] = parts;
    return [f, m, g].join("");
  }
  return named.join(" ");
}
