import { afterEach, describe, expect, it } from "vitest";

import { areas, catalogues, english } from "./catalogue";
import { LANGUAGES, fill, formatNumber, language, onLanguage, setLanguage, t, tIn } from "./index";
import type { Message } from "./types";

/** The placeholders of a message, in every form, sorted. */
function placeholders(message: Message): string[] {
  const texts = typeof message === "string" ? [message] : [message.one, message.other];
  return [...new Set(texts.flatMap((text) => [...text.matchAll(/\{(\w+)\}/g)].map((m) => m[1] ?? "")))].sort();
}

/** Texts of a message, in every form. */
function texts(message: Message): string[] {
  return typeof message === "string" ? [message] : [message.one, message.other];
}

describe("message catalogues", () => {
  it("translate every English message into Hungarian, with the same placeholders", () => {
    const missing: string[] = [];
    const mismatched: string[] = [];
    for (const [area, catalogue] of Object.entries(areas)) {
      const hu = catalogue.hu as Record<string, Message | undefined>;
      for (const [key, message] of Object.entries(catalogue.en as Record<string, Message>)) {
        const translated = hu[key];
        if (translated === undefined || texts(translated).some((text) => text.trim() === "")) {
          missing.push(`${area}: ${key}`);
          continue;
        }
        // A plural's "one" may say "one" instead of {count}; the placeholders of the
        // other form must all be there.
        const want = placeholders(typeof message === "string" ? message : message.other);
        const have = placeholders(translated);
        if (want.some((p) => !have.includes(p)) || have.some((p) => !placeholders(message).includes(p))) {
          mismatched.push(`${area}: ${key} (${want.join(", ")} / ${have.join(", ")})`);
        }
      }
    }
    expect(missing, "English messages without a Hungarian translation").toEqual([]);
    expect(mismatched, "Translations whose placeholders differ").toEqual([]);
  });

  it("keep each area's keys under its prefix, with none in two areas and none left over", () => {
    const seen = new Map<string, string>();
    for (const [area, catalogue] of Object.entries(areas)) {
      for (const key of Object.keys(catalogue.en)) {
        expect(key.startsWith(`${area}.`), `${key} is in ${area}`).toBe(true);
        expect(seen.get(key), `${key} is in two areas`).toBeUndefined();
        seen.set(key, area);
      }
      const extra = Object.keys(catalogue.hu).filter((key) => !Object.hasOwn(catalogue.en, key));
      expect(extra, `Hungarian messages of ${area} that English does not have`).toEqual([]);
    }
    expect(Object.keys(catalogues.hu).length).toBe(Object.keys(english).length);
  });

  it("use no em dashes and no emojis", () => {
    for (const lang of LANGUAGES) {
      for (const [key, message] of Object.entries(catalogues[lang] as Record<string, Message>)) {
        for (const text of texts(message)) {
          expect(text.includes("\u2014"), `${lang} ${key}`).toBe(false);
          expect(/\p{Extended_Pictographic}/u.test(text), `${lang} ${key}`).toBe(false);
        }
      }
    }
  });
});

describe("t", () => {
  afterEach(() => setLanguage("en"));

  it("fills placeholders, picks plural forms and writes numbers the language's way", () => {
    expect(fill("Levelled to {target} LUFS", { target: -14.5 }, "en")).toBe("Levelled to -14.5 LUFS");
    expect(fill("Levelled to {target} LUFS", { target: -14.5 }, "hu")).toBe("Levelled to -14,5 LUFS");
    expect(fill({ one: "one clip", other: "{count} clips" }, { count: 1 }, "en")).toBe("one clip");
    expect(fill({ one: "one clip", other: "{count} clips" }, { count: 3 }, "en")).toBe("3 clips");
    expect(fill("{name} and {missing}", { name: "A" }, "en"), "an unknown placeholder stays").toBe("A and {missing}");
    expect(formatNumber(1234.5, "en"), "never grouped").toBe("1234.5");
    expect(formatNumber(0.1 + 0.2, "hu")).toBe("0,3");
  });

  it("follows the language live and falls back to English", () => {
    const heard: string[] = [];
    const stop = onLanguage(() => heard.push(language()));
    expect(t("canvas.blur")).toBe("Blur");
    setLanguage("hu");
    expect(t("canvas.blur")).toBe("Elmosás");
    expect(t("canvas.several", { count: 3 })).toBe("3 kijelölt videó- és képklipre érvényes");
    setLanguage("xx");
    expect(language(), "an unknown language is English").toBe("en");
    setLanguage("en");
    stop();
    setLanguage("hu");
    expect(heard).toEqual(["hu", "en"]);
    expect(tIn("en", "canvas.color")).toBe("Colour");
  });
});
