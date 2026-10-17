/** A post's date as `YYYY-MM-DD` and as shown ("17 October 2026"). */
export function postDate(value: string | Date) {
  const date = new Date(value);
  return {
    iso: date.toISOString().slice(0, 10),
    shown: date.toLocaleDateString("en-GB", { day: "numeric", month: "long", year: "numeric", timeZone: "UTC" }),
  };
}
