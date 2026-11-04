import { useId } from "react";

export interface SelectOption {
  value: string;
  label: string;
  disabled?: boolean;
}

export interface SelectProps {
  label?: string;
  value: string;
  options: SelectOption[];
  /** 受控：直接回传选中的 value */
  onChange: (value: string) => void;
  disabled?: boolean;
  hint?: string;
  id?: string;
  name?: string;
}

export function Select({
  label,
  value,
  options,
  onChange,
  disabled = false,
  hint,
  id,
  name,
}: SelectProps) {
  const generatedId = useId();
  const selectId = id ?? generatedId;
  const hintId = `${selectId}-hint`;
  return (
    <div className="sa-field">
      {label ? (
        <label className="sa-field__label" htmlFor={selectId}>
          {label}
        </label>
      ) : null}
      <select
        id={selectId}
        name={name}
        className="sa-field__select"
        value={value}
        disabled={disabled}
        aria-describedby={hint ? hintId : undefined}
        onChange={(event) => onChange(event.target.value)}
      >
        {options.map((option) => (
          <option key={option.value} value={option.value} disabled={option.disabled}>
            {option.label}
          </option>
        ))}
      </select>
      {hint ? (
        <p className="sa-field__hint" id={hintId}>
          {hint}
        </p>
      ) : null}
    </div>
  );
}
