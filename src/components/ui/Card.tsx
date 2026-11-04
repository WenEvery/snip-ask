import type { ReactNode } from "react";

export interface CardProps {
  title?: ReactNode;
  description?: ReactNode;
  children?: ReactNode;
  actions?: ReactNode;
  className?: string;
}

export function Card({ title, description, children, actions, className }: CardProps) {
  const classes = ["sa-card", className].filter(Boolean).join(" ");
  return (
    <section className={classes}>
      {title || description || actions ? (
        <header className="sa-card__header">
          <div className="sa-card__heading">
            {title ? <h3 className="sa-card__title">{title}</h3> : null}
            {description ? <p className="sa-card__description">{description}</p> : null}
          </div>
          {actions ? <div className="sa-card__actions">{actions}</div> : null}
        </header>
      ) : null}
      {children ? <div className="sa-card__body">{children}</div> : null}
    </section>
  );
}
