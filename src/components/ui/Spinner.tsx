export interface SpinnerProps {
  size?: "sm" | "md" | "lg";
  label?: string;
  className?: string;
}

export function Spinner({ size = "md", label, className }: SpinnerProps) {
  const classes = ["sa-spinner", `sa-spinner--${size}`, className].filter(Boolean).join(" ");
  return (
    <span className={classes} role="status" aria-live="polite">
      <span className="sa-spinner__ring" aria-hidden="true" />
      {label ? (
        <span className="sa-spinner__label">{label}</span>
      ) : (
        <span className="sa-visually-hidden">加载中</span>
      )}
    </span>
  );
}
