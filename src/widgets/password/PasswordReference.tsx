import type { CharClass, Options, Strength } from "../../lib/password";
import { classOf, MAX_LENGTH, MIN_LENGTH, STRENGTH_STEPS } from "../../lib/password";
import { useText } from "../../lib/i18n";
import { copySelectionAsSecret } from "../../lib/copy";
import { Button } from "../../components/Button";
import { Slider } from "../../components/Slider";
import { SegmentMeter } from "../../components/SegmentMeter";
import { MetricLabel } from "../../components/MetricLabel";
import { CopyIcon, DoneIcon, FailedIcon, RefreshIcon, ShieldIcon } from "../../components/Icons";

const CLASSES: CharClass[] = ["lower", "upper", "digit", "symbol"];
const CLASS_LABEL: Record<CharClass, string> = { lower: "a", upper: "A", digit: "0", symbol: "#" };
const OPTIONAL_CLASSES = [
  { key: "upper", label: "A-Z", kind: "upper" },
  { key: "digits", label: "0-9", kind: "digit" },
  { key: "symbols", label: "!@#", kind: "symbol" },
] as const;

type Props = {
  layout: number;
  options: Options;
  value: string;
  strength: Strength;
  counts: Record<CharClass, number>;
  copied: boolean | undefined;
  onChange: <K extends keyof Options>(key: K, value: Options[K]) => void;
  onCopy: () => void;
  onGenerate: () => void;
};

/** NEW меняет только компоновку; параметры и пароль принадлежат PasswordWidget. */
export function PasswordReference({ layout, options, value, strength, counts, copied, onChange, onCopy, onGenerate }: Props) {
  const t = useText();
  const variant = layout === 1 ? 1 : 0;
  const strengthDetails = t(strength.label) + " · " + t("password.strengthDetails", { bits: strength.bits, level: strength.level });
  const result = (
    <output className="pass-reference__value" data-no-drag onCopy={copySelectionAsSecret}>
      <span className="pass-reference__chars">
        {[...value].map((character, index) => (
          <span key={index} data-class={classOf(character)}>{character}</span>
        ))}
      </span>
    </output>
  );
  const copy = (
    <span className="pass-reference__copy" data-feedback={copied === undefined ? undefined : copied ? "copied" : "failed"} data-accent={copied === false ? "danger" : undefined}>
      <Button
        variant="outline"
        icon={copied === undefined ? <CopyIcon /> : copied ? <DoneIcon /> : <FailedIcon />}
        label={copied === undefined ? t("common.copy") : copied ? t("common.copied") : t("common.failed")}
        onClick={onCopy}
      />
    </span>
  );
  const label = (
    <span className="pass-reference__strength" data-accent={strength.accent} title={strengthDetails}>
      <MetricLabel icon={<ShieldIcon />}>{t(strength.label)}</MetricLabel>
    </span>
  );
  const meter = (
    <div className="pass-reference__meter" role="meter" aria-label={t(strength.label)} aria-valuemin={0} aria-valuemax={STRENGTH_STEPS} aria-valuenow={strength.level} aria-valuetext={strengthDetails} title={strengthDetails}>
      <SegmentMeter level={strength.level} total={STRENGTH_STEPS} accent={strength.accent} />
    </div>
  );
  const length = (
    <div className="pass-reference__length">
      <span className="pass-reference__caption">{t("password.length")}</span>
      <Slider value={options.length} min={MIN_LENGTH} max={MAX_LENGTH} ariaLabel={t("password.length")} onChange={(next) => onChange("length", next)} />
      <span className="pass-reference__number tabular">{options.length}</span>
    </div>
  );
  const toggles = (
    <div className="pass-reference__toggles">
      {OPTIONAL_CLASSES.map(({ key, label: optionLabel, kind }) => (
        <button type="button" key={key} className="pass-reference__option" aria-pressed={options[key]} aria-label={optionLabel} onClick={() => onChange(key, !options[key])}>
          <span className="pass-reference__tick" aria-hidden="true">{options[key] && <DoneIcon />}</span>
          <span className="pass-reference__option-label">{optionLabel}</span>
          {variant === 1 && (
            <span className="pass-reference__option-count" data-class={kind}>
              <span className="pass-reference__class">{CLASS_LABEL[kind]}</span>
              <span className="tabular">{counts[kind]}</span>
            </span>
          )}
        </button>
      ))}
    </div>
  );
  const generate = (
    <div className="pass-reference__generate">
      <Button icon={<RefreshIcon />} label={t("password.generate")} onClick={onGenerate} />
    </div>
  );

  return (
    <div className="pass-reference" data-layout={variant}>
      {variant === 0 ? (
        <>
          <div className="pass-reference__result">{result}{copy}</div>
          <div className="pass-reference__summary">
            {label}{meter}
            <span className="pass-reference__counts">
              {CLASSES.filter((kind) => counts[kind] > 0).map((kind) => (
                <span className="pass-reference__count" key={kind} data-class={kind}>
                  <span className="pass-reference__class">{CLASS_LABEL[kind]}</span>
                  <span className="tabular">{counts[kind]}</span>
                </span>
              ))}
            </span>
          </div>
          {length}
          <div className="pass-reference__controls">{toggles}{generate}</div>
        </>
      ) : (
        <>
          <div className="pass-reference__parameters">{length}{toggles}</div>
          <div className="pass-reference__content">
            <div className="pass-reference__field">{result}{copy}</div>
            {label}{meter}
            {generate}
          </div>
        </>
      )}
    </div>
  );
}
