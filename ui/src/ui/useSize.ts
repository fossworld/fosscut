import { type RefObject, useLayoutEffect, useState } from "react";

/** The size of the element `ref` holds, in CSS pixels, following it as it changes. */
export function useSize(ref: RefObject<Element | null>): { width: number; height: number } {
  const [size, setSize] = useState({ width: 0, height: 0 });
  useLayoutEffect(() => {
    const element = ref.current;
    if (!element) return;
    // The layout size, which transforms of the page do not change.
    const observer = new ResizeObserver(([entry]) => {
      const rect = entry?.contentRect;
      if (!rect) return;
      setSize((was) => (was.width === rect.width && was.height === rect.height ? was : { width: rect.width, height: rect.height }));
    });
    observer.observe(element);
    return () => observer.disconnect();
  }, [ref]);
  return size;
}
