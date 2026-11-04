import { useId } from "react";

export interface TextFieldProps {
  label?: string;
  value: string;
  /** 受控：直接回传字符串，调用方无需处理事件对象 */
  onChange: (value: string) => void;
  placeholder?: string;
  type?: "text" | "password";
  hint?: string;
  error?: string;
  disabled?: boolean;
  monospace?: boolean;
  id?: string;
  name?: string;
}

export function TextField({
  label,
  value,
  onChange,
  placeholder,
  type = "text",
  hint,
  error,
  disabled = false,
  monospace = false,
  id,
  name,
}: TextFieldProps) {
  const generatedId = useId();
  const inputId = id ?? generatedId;
  const hintId = `${inputId}-hint`;
  const errorId = `${inputId}-error`;
  const describedBy = error ? errorId : hint ? hintId : undefined;
  return (
    <div className={`sa-field${error ? " sa-field--error" : ""}`}>
      {label ? (
        <label className="sa-field__label" htmlFor={inputId}>
          {label}
        </label>
      ) : null}
      <input
        id={inputId}
        name={name}
        className={`sa-field__input${monospace ? " sa-field__input--mono" : ""}`}
        type={type}
        value={value}
        placeholder={placeholder}
        disabled={disabled}
        aria-invalid={error ? true : undefined}
        aria-describedby={describedBy}
        onChange={(event) => onChange(event.target.value)}
      />
      {hint && !error ? (
        <p className="sa-field__hint" id={hintId}>
          {hint}
        </p>
      ) : null}
      {error ? (
        <p className="sa-field__error" id={errorId} role="alert">
          {error}
        </p>
      ) : null}
    </div>
  );
}
