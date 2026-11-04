import type { ReactNode } from "react";

export interface EmptyStateProps {
  icon?: ReactNode;
  title: string;
  description?: string;
  children?: ReactNode;
}

export function EmptyState({ icon, title, description, children }: EmptyStateProps) {
  return (
    <div className="sa-empty">
      {icon ? (
        <div className="sa-empty__icon" aria-hidden="true">
          {icon}
        </div>
      ) : null}
      <p className="sa-empty__title">{title}</p>
      {description ? <p className="sa-empty__desc">{description}</p> : null}
      {children}
    </div>
  );
}
