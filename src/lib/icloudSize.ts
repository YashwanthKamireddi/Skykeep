/**
 * Sizes of iCloud storage, as Apple writes them on icloud.com and the iPhone: in thousands, not
 * 1024s, so a 200 GB plan reads "200 GB". kB with no decimals, MB with one, GB and up with two.
 * Every figure of Apple's storage goes through here, so Drive and Skykeep's window say the same.
 */
export function icloudSize(bytes: number): string {
  const units = ["bytes", "kB", "MB", "GB", "TB", "PB"];
  const places = [0, 0, 1, 2, 2, 2];
  let value = bytes;
  let unit = 0;
  while (value >= 1000 && unit < units.length - 1) {
    value /= 1000;
    unit += 1;
  }
  const p = 10 ** places[unit];
  return `${(Math.round(value * p) / p).toLocaleString()} ${units[unit]}`;
}
