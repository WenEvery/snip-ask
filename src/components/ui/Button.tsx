import type { ReactNode } from "react";

export interface ButtonProps {
  children: ReactNode;
  /** 视觉样式，默认 secondary */
  variant?: "primary" | "secondary" | "ghost" | "danger";
  size?: "sm" | "md";
  disabled?: boolean;
  onClick?: () => void;
  title?: string;
  type?: "button" | "submit" | "reset";
  className?: string;
}

export function Button({
  children,
  variant = "secondary",
  size = "md",
  disabled = false,
  onClick,
  title,
  type = "button",
  className,
}: ButtonProps) {
  const classes = ["sa-btn", `sa-btn--${variant}`, `sa-btn--${size}`, className]
    .filter(Boolean)
    .join(" ");
  return (
    <button type={type} className={classes} disabled={disabled} onClick={onClick} title={title}>
      {children}
    </button>
  );
}
