// The shapes of message catalogues: each area of the interface has one module in
// ./messages exporting `en`, its English messages, and `hu`, the same keys in Hungarian.

/** A message with a form for one and a form for any other count (`{count}`). */
export interface Plural {
  one: string;
  other: string;
}

/**
 * A message: text with `{name}` placeholders filled from the parameters, or a plural.
 * Hungarian nouns keep their singular after a number, so Hungarian rarely needs one.
 */
export type Message = string | Plural;

/** A catalogue of messages by key. */
export type Messages = Record<string, Message>;

/** The same keys as the English catalogue `T`, in another language. */
export type Translation<T extends Messages> = { [K in keyof T]: Message };

/** Values for a message's placeholders; numbers are written the language's way. */
export type Params = Record<string, string | number>;
