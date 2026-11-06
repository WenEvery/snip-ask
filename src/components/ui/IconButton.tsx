import type { ReactNode } from "react";

export interface IconButtonProps {
  /** 无障碍名称（必填）：等价于 aria-label */
  label: string;
  title?: string;
  onClick?: () => void;
  disabled?: boolean;
  children: ReactNode;
  className?: string;
}

export function IconButton({
  label,
  title,
  onClick,
  disabled = false,
  children,
  className,
}: IconButtonProps) {
  const classes = ["sa-icon-btn", className].filter(Boolean).join(" ");
  return (
    <button
      type="button"
      className={classes}
      aria-label={label}
      title={title ?? label}
      disabled={disabled}
      onClick={onClick}
    >
      {children}
    </button>
  );
}
