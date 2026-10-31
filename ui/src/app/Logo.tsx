import { ChevronDown } from "lucide-react";
import { useId } from "react";

import { Glyph } from "../ui/controls";

/** FossCut's mark: a play triangle cut through on an orange tile. */
export function Mark({ size = 24 }: { size?: number }) {
  const id = useId().replace(/:/g, "");
  return (
    <svg className="mark" viewBox="0 0 24 24" width={size} height={size} aria-hidden="true" focusable="false">
      <defs>
        <linearGradient id={`${id}-tile`} x1="0" y1="0" x2="1" y2="1">
          <stop offset="0" stopColor="var(--orange-400)" />
          <stop offset="1" stopColor="var(--orange-600)" />
        </linearGradient>
        <mask id={`${id}-cut`}>
          <rect width="24" height="24" fill="#fff" />
          <path d="M5.5 16.4 18.6 7.6" stroke="#000" strokeWidth="1.9" strokeLinecap="round" />
        </mask>
      </defs>
      <rect width="24" height="24" rx="6.5" fill={`url(#${id}-tile)`} />
      <path d="M9 6.6c0-.8.9-1.3 1.6-.9l7.4 4.6c.7.4.7 1.4 0 1.8l-7.4 4.6c-.7.4-1.6-.1-1.6-.9z" fill="#fff" mask={`url(#${id}-cut)`} />
    </svg>
  );
}

/** The app menu's button face: the mark and a small arrow. */
export function Logo() {
  return (
    <>
      <Mark size={24} />
      <Glyph icon={ChevronDown} size="sm" className="chevron" />
    </>
  );
}
