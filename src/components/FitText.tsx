import { useLayoutEffect, useRef, useState } from "react";
import { cn } from "@/lib/utils";

/**
 * Single-line text that never wraps or truncates: when it doesn't fit the
 * space it is given, it disappears (keeping its slot so it can come back).
 */
export function FitText({ children, className }: { children: React.ReactNode; className?: string }) {
  const ref = useRef<HTMLSpanElement>(null);
  const [fits, setFits] = useState(true);
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const check = () => setFits(el.scrollWidth <= el.clientWidth + 0.5);
    check();
    const ro = new ResizeObserver(check);
    ro.observe(el);
    return () => ro.disconnect();
  }, [children]);
  return (
    <span ref={ref} className={cn("block min-w-0 flex-1 overflow-hidden whitespace-nowrap text-left", !fits && "invisible", className)} aria-hidden={!fits || undefined}>
      {children}
    </span>
  );
}
