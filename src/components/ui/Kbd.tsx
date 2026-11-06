import type { ReactNode } from "react";

export interface KbdProps {
  children: ReactNode;
  title?: string;
}

/** 键帽展示，例如 <Kbd>Ctrl</Kbd><Kbd>Q</Kbd> */
export function Kbd({ children, title }: KbdProps) {
  return (
    <kbd className="sa-kbd" title={title}>
      {children}
    </kbd>
  );
}
