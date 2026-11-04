/** 引导页可选卡片：供应商 / 翻译档位 / 翻译后端 */

import type { ReactNode } from "react";

export interface OptionCardProps {
  selected: boolean;
  title: string;
  description?: string;
  children?: ReactNode;
  onSelect: () => void;
  disabled?: boolean;
}

export function OptionCard(props: OptionCardProps) {
  return (
    <button
      type="button"
      className={`ob-card${props.selected ? " is-selected" : ""}`}
      aria-pressed={props.selected}
      disabled={props.disabled}
      onClick={props.onSelect}
    >
      <span className="ob-card-head">
        <span className="ob-card-title">{props.title}</span>
        {props.selected ? <span className="ob-card-check">✓</span> : null}
      </span>
      {props.description ? <span className="ob-card-desc">{props.description}</span> : null}
      {props.children ? <span className="ob-card-body">{props.children}</span> : null}
    </button>
  );
}
