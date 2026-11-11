import { forwardRef } from "react";
import { Search, X } from "lucide-react";
import { useI18n } from "../lib/i18n";

type Props = Omit<React.InputHTMLAttributes<HTMLInputElement>, "value" | "onChange" | "type" | "className"> & {
  value: string;
  onChange: (value: string) => void;
  /** Given, a ✕ clears what is typed. */
  onClear?: () => void;
  /** In a toolbar, it takes the toolbar's capsule beside the other controls. */
  inToolbar?: boolean;
  className?: string;
};

/**
 * The search field, one shape everywhere: a capsule with the magnifier, the words, and a ✕
 * once something is typed. In a sidebar or a list it is a soft fill; in a toolbar it is one
 * more capsule of the toolbar.
 */
export const SearchField = forwardRef<HTMLInputElement, Props>(function SearchField(
  { value, onChange, onClear, inToolbar, className, spellCheck = false, ...input },
  ref,
) {
  const { t } = useI18n();
  return (
    <label className={`search-field${inToolbar ? " toolbar-group" : ""}${className ? ` ${className}` : ""}`}>
      <Search size={14} strokeWidth={2.2} aria-hidden="true" />
      <input ref={ref} type="search" value={value} spellCheck={spellCheck} onChange={(e) => onChange(e.target.value)} {...input} />
      {value && onClear && (
        <button type="button" className="search-clear" aria-label={t("common.clearSearch")} onClick={onClear}>
          <X size={10} strokeWidth={3} aria-hidden="true" />
        </button>
      )}
    </label>
  );
});
