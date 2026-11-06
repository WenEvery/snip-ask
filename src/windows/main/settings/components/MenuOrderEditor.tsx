/** 动作菜单顺序编辑：勾选显隐 + 上移/下移调整顺序（数组顺序即菜单顺序） */

import type { ActionId } from "@/shared/contract/types";
import { t } from "@/i18n";
import { ACTION_LABELS } from "../presets";

export interface MenuOrderEditorProps {
  title: string;
  description: string;
  candidates: ActionId[];
  value: ActionId[];
  onChange: (next: ActionId[]) => void;
}

export function MenuOrderEditor(props: MenuOrderEditorProps) {
  const { value, candidates, onChange } = props;

  const toggle = (action: ActionId) => {
    if (value.includes(action)) {
      onChange(value.filter((item) => item !== action));
    } else {
      onChange([...value, action]);
    }
  };

  const move = (index: number, delta: number) => {
    const target = index + delta;
    if (target < 0 || target >= value.length) return;
    const next = [...value];
    const item = next[index];
    if (item === undefined) return;
    next.splice(index, 1);
    next.splice(target, 0, item);
    onChange(next);
  };

  return (
    <div className="sa-menu-editor">
      <div className="sa-menu-head">
        <h4 className="sa-menu-title">{props.title}</h4>
        <p className="sa-field-hint">{props.description}</p>
      </div>
      <div className="sa-menu-order">
        {value.length === 0 ? (
          <p className="sa-hint">{t("全部隐藏：该选区下动作菜单将为空。")}</p>
        ) : (
          value.map((action, index) => (
            <div key={action} className="sa-menu-item">
              <span className="sa-menu-index">{index + 1}</span>
              <span className="sa-menu-label">{ACTION_LABELS[action]}</span>
              <button
                type="button"
                className="sa-linkbtn"
                disabled={index === 0}
                onClick={() => move(index, -1)}
              >
                {t("上移")}
              </button>
              <button
                type="button"
                className="sa-linkbtn"
                disabled={index === value.length - 1}
                onClick={() => move(index, 1)}
              >
                {t("下移")}
              </button>
              <button type="button" className="sa-linkbtn is-danger" onClick={() => toggle(action)}>
                {t("隐藏")}
              </button>
            </div>
          ))
        )}
        <div className="sa-menu-candidates">
          {candidates.map((action) => {
            const included = value.includes(action);
            return (
              <button
                key={action}
                type="button"
                className={`sa-chip${included ? " is-on" : ""}`}
                title={included ? t("点击从菜单隐藏") : t("点击加入菜单")}
                onClick={() => toggle(action)}
              >
                {included ? "✓ " : "+ "}
                {ACTION_LABELS[action]}
              </button>
            );
          })}
        </div>
      </div>
    </div>
  );
}
