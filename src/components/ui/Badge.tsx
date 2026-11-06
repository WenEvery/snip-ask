import type { ReactNode } from "react";

export type BadgeTone = "neutral" | "accent" | "success" | "warning" | "danger";

export interface BadgeProps {
  children: ReactNode;
  tone?: BadgeTone;
  title?: string;
}

export function Badge({ children, tone = "neutral", title }: BadgeProps) {
  return (
    <span className={`sa-badge sa-badge--${tone}`} title={title}>
      {children}
    </span>
  );
}
