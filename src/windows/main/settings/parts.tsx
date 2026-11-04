/**
 * 设置页局部基础组件：在 `@/components/ui` 之上做一层薄封装。
 * 这里只放表单控件；布局与反馈类组件见 `layout.tsx` / `feedback.tsx`，
 * 通过本文件统一再导出，分组代码只依赖 `../parts`。
 */

import { useEffect, useRef, useState } from "react";
import { Select, Switch, TextField } from "@/components/ui";

export * from "./layout";
export * from "./feedback";

/* ───────────────────────── 表单控件 ───────────────────────── */

export interface TextInputProps {
  label?: string;
  value: string;
  onChange: (value: string) => void;
  placeholder?: string;
  type?: "text" | "password";
  hint?: string;
  error?: string;
  disabled?: boolean;
  monospace?: boolean;
}

export function TextInput(props: TextInputProps) {
  return (
    <TextField
      label={props.label}
      value={props.value}
      onChange={props.onChange}
      placeholder={props.placeholder}
      type={props.type ?? "text"}
      hint={props.hint}
      error={props.error}
      disabled={props.disabled}
      monospace={props.monospace}
    />
  );
}

export interface SelectFieldProps {
  label?: string;
  value: string;
  options: { value: string; label: string; disabled?: boolean }[];
  onChange: (value: string) => void;
  hint?: string;
  disabled?: boolean;
}

export function SelectField(props: SelectFieldProps) {
  return (
    <Select
      label={props.label}
      value={props.value}
      options={props.options}
      onChange={props.onChange}
      hint={props.hint}
      disabled={props.disabled}
    />
  );
}

export interface SwitchRowProps {
  label: string;
  description?: string;
  checked: boolean;
  onChange: (checked: boolean) => void;
  disabled?: boolean;
}

export function SwitchRow(props: SwitchRowProps) {
  return (
    <Switch
      checked={props.checked}
      onChange={props.onChange}
      label={props.label}
      description={props.description}
      disabled={props.disabled}
    />
  );
}

export interface NumberFieldProps {
  label?: string;
  value: number;
  onChange: (value: number) => void;
  min?: number;
  max?: number;
  step?: number;
  suffix?: string;
  hint?: string;
  disabled?: boolean;
}

/** 数字输入：本地保留文本缓冲，失焦或回车时校验并提交，避免边输入边夹紧 */
export function NumberField(props: NumberFieldProps) {
  const { value, onChange, min, max } = props;
  const [text, setText] = useState(String(value));
  const lastValue = useRef(value);

  useEffect(() => {
    if (value !== lastValue.current) {
      lastValue.current = value;
      setText(String(value));
    }
  }, [value]);

  const commit = () => {
    const parsed = Number(text);
    if (!Number.isFinite(parsed)) {
      setText(String(value));
      return;
    }
    let next = parsed;
    if (typeof min === "number") next = Math.max(min, next);
    if (typeof max === "number") next = Math.min(max, next);
    lastValue.current = next;
    setText(String(next));
    if (next !== value) onChange(next);
  };

  const range = `${typeof min === "number" ? min : "—"} ~ ${typeof max === "number" ? max : "—"}`;

  return (
    <label className="sa-numfield">
      {props.label ? <span className="sa-numfield-label">{props.label}</span> : null}
      <span className="sa-numfield-inputwrap">
        <input
          className="sa-numfield-input"
          type="number"
          inputMode="numeric"
          value={text}
          min={min}
          max={max}
          step={props.step ?? 1}
          disabled={props.disabled}
          onChange={(event) => setText(event.target.value)}
          onBlur={commit}
          onKeyDown={(event) => {
            if (event.key === "Enter") commit();
          }}
        />
        {props.suffix ? <span className="sa-numfield-suffix">{props.suffix}</span> : null}
      </span>
      <span className="sa-numfield-hint">{props.hint ?? `范围 ${range}`}</span>
    </label>
  );
}

export interface SliderFieldProps {
  label: string;
  value: number;
  min: number;
  max: number;
  step: number;
  onChange: (value: number) => void;
  format?: (value: number) => string;
  hint?: string;
}

export function SliderField(props: SliderFieldProps) {
  const format = props.format ?? ((value: number) => String(value));
  return (
    <div className="sa-slider">
      <div className="sa-slider-head">
        <span className="sa-field-label">{props.label}</span>
        <span className="sa-slider-value">{format(props.value)}</span>
      </div>
      <input
        className="sa-slider-input"
        type="range"
        min={props.min}
        max={props.max}
        step={props.step}
        value={props.value}
        onChange={(event) => props.onChange(Number(event.target.value))}
      />
      {props.hint ? <span className="sa-field-hint">{props.hint}</span> : null}
    </div>
  );
}

/** 一组互斥选项按钮（主题 / 字号 / 云本地等） */
export interface SegmentOption<T extends string> {
  value: T;
  label: string;
  description?: string;
}

export interface SegmentedControlProps<T extends string> {
  value: T;
  options: SegmentOption<T>[];
  onChange: (value: T) => void;
  disabled?: boolean;
}

export function SegmentedControl<T extends string>(props: SegmentedControlProps<T>) {
  return (
    <div className="sa-segmented" role="radiogroup">
      {props.options.map((option) => (
        <button
          key={option.value}
          type="button"
          role="radio"
          aria-checked={option.value === props.value}
          className={`sa-seg${option.value === props.value ? " is-active" : ""}`}
          disabled={props.disabled}
          title={option.description}
          onClick={() => props.onChange(option.value)}
        >
          <span className="sa-seg-label">{option.label}</span>
          {option.description ? <span className="sa-seg-desc">{option.description}</span> : null}
        </button>
      ))}
    </div>
  );
}
