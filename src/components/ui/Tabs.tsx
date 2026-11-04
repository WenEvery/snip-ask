export interface TabItem {
  id: string;
  label: string;
}

export interface TabsProps {
  items: TabItem[];
  active: string;
  onChange: (id: string) => void;
}

export function Tabs({ items, active, onChange }: TabsProps) {
  return (
    <div className="sa-tabs" role="tablist">
      {items.map((item) => {
        const selected = item.id === active;
        return (
          <button
            key={item.id}
            type="button"
            role="tab"
            aria-selected={selected}
            className={`sa-tabs__item${selected ? " sa-tabs__item--active" : ""}`}
            onClick={() => onChange(item.id)}
          >
            {item.label}
          </button>
        );
      })}
    </div>
  );
}
