import { describe, expect, it } from "vitest";

import { THEME_CHOICES, parseChoice, resolveTheme } from "./theme";

describe("theme", () => {
  it("is dark unless light or the system's is chosen", () => {
    expect(parseChoice(null)).toBe("dark");
    expect(parseChoice("purple")).toBe("dark");
    expect(parseChoice("light")).toBe("light");
    expect(parseChoice("system")).toBe("system");
    expect(THEME_CHOICES).toEqual(["dark", "light", "system"]);
  });

  it("follows the system only when asked to", () => {
    expect(resolveTheme("system", true)).toBe("light");
    expect(resolveTheme("system", false)).toBe("dark");
    expect(resolveTheme("dark", true)).toBe("dark");
    expect(resolveTheme("light", false)).toBe("light");
  });
});
