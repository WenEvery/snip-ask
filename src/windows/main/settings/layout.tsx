/** 设置页布局类局部组件（分组外壳 / 面板 / 字段行 / 提示） */

import type { ReactNode } from "react";
import { t } from "@/i18n";
import { Spinner } from "@/components/ui";

export interface SectionShellProps {
  title: string;
  description?: string;
  actions?: ReactNode;
  children: ReactNode;
}

export function SectionShell(props: SectionShellProps) {
  return (
    <section className="sa-section">
      <header className="sa-section-head">
        <div>
          <h2 className="sa-section-title">{props.title}</h2>
          {props.description ? <p className="sa-section-desc">{props.description}</p> : null}
        </div>
        {props.actions ? <div className="sa-section-actions">{props.actions}</div> : null}
      </header>
      <div className="sa-section-body">{props.children}</div>
    </section>
  );
}

export interface PanelProps {
  title?: string;
  description?: string;
  actions?: ReactNode;
  children: ReactNode;
}

export function Panel(props: PanelProps) {
  return (
    <div className="sa-panel">
      {props.title || props.actions ? (
        <div className="sa-panel-head">
          <div>
            {props.title ? <h3 className="sa-panel-title">{props.title}</h3> : null}
            {props.description ? <p className="sa-panel-desc">{props.description}</p> : null}
          </div>
          {props.actions ? <div className="sa-panel-actions">{props.actions}</div> : null}
        </div>
      ) : null}
      <div className="sa-panel-body">{props.children}</div>
    </div>
  );
}

export interface FieldProps {
  label?: string;
  hint?: string;
  children: ReactNode;
  /** 控件与标签上下排布 */
  stacked?: boolean;
}

export function Field(props: FieldProps) {
  return (
    <div className={`sa-field${props.stacked ? " is-stacked" : ""}`}>
      {props.label ? <span className="sa-field-label">{props.label}</span> : null}
      <div className="sa-field-control">{props.children}</div>
      {props.hint ? <span className="sa-field-hint">{props.hint}</span> : null}
    </div>
  );
}

export function InfoRow(props: { label: string; value: ReactNode; mono?: boolean; hint?: string }) {
  return (
    <div className="sa-inforow">
      <span className="sa-inforow-label">{props.label}</span>
      <span className={`sa-inforow-value${props.mono ? " is-mono" : ""}`}>{props.value}</span>
      {props.hint ? <span className="sa-inforow-hint">{props.hint}</span> : null}
    </div>
  );
}

export function Hint(props: { children: ReactNode }) {
  return <p className="sa-hint">{props.children}</p>;
}

export function WarnText(props: { children: ReactNode }) {
  return <p className="sa-warn">{props.children}</p>;
}

export function LoadingBlock(props: { text?: string }) {
  return (
    <div className="sa-loading">
      <Spinner />
      <span>{props.text ?? t("加载中…")}</span>
    </div>
  );
}

export function EmptyHint(props: { children: ReactNode }) {
  return <div className="sa-empty">{props.children}</div>;
}
