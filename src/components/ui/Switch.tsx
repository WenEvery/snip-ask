import { useId } from "react";

export interface SwitchProps {
  checked: boolean;
  /** 受控：直接回传布尔值 */
  onChange: (checked: boolean) => void;
  label?: string;
  description?: string;
  disabled?: boolean;
  id?: string;
}

export function Switch({
  checked,
  onChange,
  label,
  description,
  disabled = false,
  id,
}: SwitchProps) {
  const generatedId = useId();
  const inputId = id ?? generatedId;
  return (
    <div className={`sa-switch${disabled ? " sa-switch--disabled" : ""}`}>
      <input
        id={inputId}
        className="sa-switch__input"
        type="checkbox"
        role="switch"
        checked={checked}
        disabled={disabled}
        aria-describedby={description ? `${inputId}-desc` : undefined}
        onChange={(event) => onChange(event.target.checked)}
      />
      {label || description ? (
        <div className="sa-switch__text">
          {label ? (
            <label className="sa-switch__label" htmlFor={inputId}>
              {label}
            </label>
          ) : null}
          {description ? (
            <p className="sa-switch__desc" id={`${inputId}-desc`}>
              {description}
            </p>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}
