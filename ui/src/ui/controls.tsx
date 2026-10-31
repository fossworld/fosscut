// The interface's small building blocks: icon buttons, tabs, segmented choices, menus,
// sliders with numbers and section headings. Styled by ../styles.css from the design tokens;
// every text they show is passed in, already translated.

import { ChevronLeft, ChevronRight, type LucideIcon } from "lucide-react";
import { type ReactNode, useEffect, useId, useLayoutEffect, useRef, useState } from "react";

import { type ActionId, hint } from "../shortcuts";

/** Joins class names, leaving out the empty ones. */
export function cx(...names: (string | false | null | undefined)[]): string {
  return names.filter(Boolean).join(" ");
}

/** The size icons are drawn at, by the size of what holds them. */
export const ICON = { sm: 14, md: 16, lg: 18 } as const;

/** An icon from the icon set, at a size, hidden from screen readers (its button is named). */
export function Glyph({ icon: Icon, size = "md", className }: { icon: LucideIcon; size?: keyof typeof ICON; className?: string }) {
  return <Icon size={ICON[size]} strokeWidth={1.8} aria-hidden="true" focusable="false" className={className} />;
}

/**
 * A button that shows an icon, named by `label` for screen readers and in its tooltip,
 * with the shortcut of `action` when given. Children show beside the icon.
 */
export function IconButton({
  icon,
  label,
  action,
  onClick,
  pressed,
  disabled,
  variant,
  size = "md",
  className,
  children,
  tooltip,
}: {
  icon: LucideIcon;
  label: string;
  action?: ActionId;
  onClick?: (event: React.MouseEvent<HTMLButtonElement>) => void;
  pressed?: boolean;
  disabled?: boolean;
  variant?: "ghost" | "primary" | "danger" | "solid";
  size?: "sm" | "md" | "lg";
  className?: string;
  children?: ReactNode;
  /** A longer explanation for the tooltip; the label when omitted. */
  tooltip?: string;
}) {
  const title = action ? hint(tooltip ?? label, action) : (tooltip ?? label);
  return (
    <button
      type="button"
      className={cx("icon-button", variant ?? "ghost", size, children ? "with-text" : null, pressed && "pressed", className)}
      aria-label={children ? undefined : label}
      title={title}
      aria-pressed={pressed}
      disabled={disabled}
      onClick={onClick}
    >
      <Glyph icon={icon} size={size === "lg" ? "lg" : size === "sm" ? "sm" : "md"} />
      {children}
    </button>
  );
}

/**
 * Which ends of a strip that scrolls sideways have more past them, followed as it scrolls
 * and resizes; the wheel scrolls it sideways too.
 */
/**
 * Whether a strip scrolled to `left`, `width` wide with `content` of content, continues past
 * its start and its end; a pixel of rounding counts as neither.
 */
export function stripEnds(left: number, width: number, content: number): { start: boolean; end: boolean } {
  return { start: left > 1, end: left + width < content - 1 };
}

export function useStripEnds(strip: React.RefObject<HTMLDivElement | null>): { start: boolean; end: boolean } {
  const [ends, setEnds] = useState({ start: false, end: false });
  useLayoutEffect(() => {
    const element = strip.current;
    if (!element) return;
    const measure = () => {
      const { start, end } = stripEnds(element.scrollLeft, element.clientWidth, element.scrollWidth);
      setEnds((was) => (was.start === start && was.end === end ? was : { start, end }));
    };
    const wheel = (event: WheelEvent) => {
      if (Math.abs(event.deltaY) <= Math.abs(event.deltaX)) return;
      event.preventDefault();
      element.scrollLeft += event.deltaY;
    };
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    element.addEventListener("scroll", measure);
    element.addEventListener("wheel", wheel, { passive: false });
    measure();
    return () => {
      observer.disconnect();
      element.removeEventListener("scroll", measure);
      element.removeEventListener("wheel", wheel);
    };
  }, [strip]);
  return ends;
}

/** A row of tabs; the panel below is the caller's, labelled by the tab's id. */
export function Tabs<T extends string>({
  tabs,
  value,
  onChange,
  label,
  scroll,
  className,
}: {
  tabs: { id: T; label: string; icon?: LucideIcon; help?: string }[];
  value: T;
  onChange: (id: T) => void;
  label: string;
  /** The names of the buttons that scroll the tabs when they do not all fit. */
  scroll: ScrollLabels;
  className?: string;
}) {
  const strip = useRef<HTMLDivElement>(null);
  const ends = useStripEnds(strip);
  // The chosen tab stays in sight when the strip scrolls.
  useEffect(() => {
    const active = strip.current?.querySelector<HTMLElement>('[aria-selected="true"]');
    active?.scrollIntoView?.({ block: "nearest", inline: "nearest" });
  }, [value]);
  const move = (event: React.KeyboardEvent, index: number) => {
    const step = event.key === "ArrowRight" ? 1 : event.key === "ArrowLeft" ? -1 : 0;
    if (step === 0) return;
    event.preventDefault();
    event.stopPropagation();
    const next = tabs[(index + step + tabs.length) % tabs.length];
    if (!next) return;
    onChange(next.id);
    strip.current?.querySelectorAll<HTMLElement>('[role="tab"]')[(index + step + tabs.length) % tabs.length]?.focus();
  };
  return (
    <StripBar strip={strip} ends={ends} labels={scroll}>
      <div className={cx("tabs", className, ends.start && "more-start", ends.end && "more-end")} role="tablist" aria-label={label} ref={strip}>
        {tabs.map((tab, index) => (
          <button
            key={tab.id}
            type="button"
            role="tab"
            id={`tab-${tab.id}`}
            aria-selected={tab.id === value}
            tabIndex={tab.id === value ? 0 : -1}
            className={cx("tab", tab.id === value && "active")}
            title={tab.help ?? tab.label}
            onClick={() => onChange(tab.id)}
            onKeyDown={(event) => move(event, index)}
          >
            {tab.icon && <Glyph icon={tab.icon} size="sm" />}
            <span>{tab.label}</span>
          </button>
        ))}
      </div>
    </StripBar>
  );
}

/** The names of a strip's scroll buttons. */
export type ScrollLabels = { earlier: string; later: string };

/**
 * A strip that scrolls sideways, with buttons at the ends it continues past, so items out of
 * sight are never hidden without a sign.
 */
export function StripBar({
  strip,
  ends,
  labels,
  children,
}: {
  strip: React.RefObject<HTMLDivElement | null>;
  ends: { start: boolean; end: boolean };
  /** The names of the buttons that scroll back and on. */
  labels: ScrollLabels;
  children: ReactNode;
}) {
  const scroll = (direction: number) => {
    const element = strip.current;
    element?.scrollBy({ left: direction * element.clientWidth * 0.8, behavior: "smooth" });
  };
  return (
    <div className="strip-bar">
      {ends.start && <IconButton icon={ChevronLeft} label={labels.earlier} size="sm" className="strip-scroll start" onClick={() => scroll(-1)} />}
      {children}
      {ends.end && <IconButton icon={ChevronRight} label={labels.later} size="sm" className="strip-scroll end" onClick={() => scroll(1)} />}
    </div>
  );
}

/** A choice of a few options side by side. */
export function Segmented<T extends string>({
  options,
  value,
  onChange,
  label,
}: {
  options: { id: T; label: string; icon?: LucideIcon; help?: string }[];
  value: T;
  onChange: (id: T) => void;
  label: string;
}) {
  return (
    <div className="segmented" role="radiogroup" aria-label={label}>
      {options.map((option) => (
        <button
          key={option.id}
          type="button"
          role="radio"
          aria-checked={option.id === value}
          className={option.id === value ? "active" : undefined}
          title={option.help ?? option.label}
          aria-label={option.icon ? option.label : undefined}
          onClick={() => onChange(option.id)}
        >
          {option.icon ? <Glyph icon={option.icon} size="sm" /> : option.label}
        </button>
      ))}
    </div>
  );
}

/** One entry of a menu. */
export interface MenuItem {
  id: string;
  label: string;
  icon?: LucideIcon;
  /** Shown at the right, such as a shortcut or a value. */
  detail?: string;
  disabled?: boolean;
  checked?: boolean;
  /** A line above this entry. */
  separated?: boolean;
  onSelect: () => void;
}

/**
 * A button that opens a list of actions below it; Escape or a click elsewhere closes it,
 * and the arrow keys move between its entries.
 */
export function Menu({
  label,
  icon,
  items,
  children,
  align = "start",
  className,
  buttonClassName,
  disabled,
}: {
  label: string;
  icon?: LucideIcon;
  items: MenuItem[];
  /** What the button shows instead of only the icon. */
  children?: ReactNode;
  align?: "start" | "end";
  className?: string;
  buttonClassName?: string;
  disabled?: boolean;
}) {
  const [open, setOpen] = useState(false);
  const box = useRef<HTMLDivElement>(null);
  const id = useId();
  useEffect(() => {
    if (!open) return;
    const away = (event: PointerEvent) => {
      if (!box.current?.contains(event.target as Node)) setOpen(false);
    };
    window.addEventListener("pointerdown", away);
    box.current?.querySelector<HTMLElement>('[role="menuitem"]:not(:disabled), [role="menuitemcheckbox"]:not(:disabled)')?.focus();
    return () => window.removeEventListener("pointerdown", away);
  }, [open]);
  const keys = (event: React.KeyboardEvent) => {
    if (event.key === "Escape") {
      event.stopPropagation();
      setOpen(false);
      box.current?.querySelector<HTMLElement>("button")?.focus();
      return;
    }
    if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
    event.preventDefault();
    event.stopPropagation();
    const entries = [...(box.current?.querySelectorAll<HTMLElement>('[role^="menuitem"]:not(:disabled)') ?? [])];
    const at = entries.indexOf(document.activeElement as HTMLElement);
    const next = entries[(at + (event.key === "ArrowDown" ? 1 : -1) + entries.length) % entries.length];
    next?.focus();
  };
  return (
    <div className={cx("menu", className)} ref={box} onKeyDown={keys}>
      <button
        type="button"
        className={cx("menu-button", children ? "with-text" : "icon-button ghost", buttonClassName)}
        aria-haspopup="menu"
        aria-expanded={open}
        aria-controls={open ? id : undefined}
        aria-label={children ? undefined : label}
        title={label}
        disabled={disabled}
        onClick={() => setOpen(!open)}
      >
        {icon && <Glyph icon={icon} />}
        {children}
      </button>
      {open && (
        <div className={cx("menu-list", `align-${align}`)} role="menu" id={id} aria-label={label}>
          {items.map((item) => (
            <button
              key={item.id}
              type="button"
              role={item.checked === undefined ? "menuitem" : "menuitemcheckbox"}
              aria-checked={item.checked}
              className={cx("menu-item", item.separated && "separated")}
              disabled={item.disabled}
              onClick={() => {
                setOpen(false);
                item.onSelect();
              }}
            >
              <span className="menu-icon">{item.icon ? <Glyph icon={item.icon} size="sm" /> : item.checked ? <span className="check" /> : null}</span>
              <span className="menu-label">{item.label}</span>
              {item.detail && <span className="menu-detail">{item.detail}</span>}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

/** A small heading over a group of fields, with optional actions at its right. */
export function Section({ title, children, actions, className }: { title: string; children: ReactNode; actions?: ReactNode; className?: string }) {
  return (
    <section className={cx("group", className)}>
      <header className="group-head">
        <h3>{title}</h3>
        {actions && <span className="group-actions">{actions}</span>}
      </header>
      {children}
    </section>
  );
}

/** A centred note for a panel with nothing to show, with an icon and an optional action. */
export function Empty({ icon, title, children }: { icon: LucideIcon; title: string; children?: ReactNode }) {
  return (
    <div className="empty-state">
      <span className="empty-icon">
        <Glyph icon={icon} size="lg" />
      </span>
      <p className="empty-title">{title}</p>
      {children}
    </div>
  );
}
