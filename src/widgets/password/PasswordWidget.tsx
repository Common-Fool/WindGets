import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  classOf,
  composition,
  generate,
  strengthOf,
  MAX_LENGTH,
  MIN_LENGTH,
  STRENGTH_STEPS,
  type CharClass,
  type Options,
} from "../../lib/password";
import { copyToClipboard } from "../../lib/copy";
import { useWindowDrag } from "../../lib/drag";
import { useText } from "../../lib/i18n";
import { useLook } from "../../lib/look";
import { useWidgetLayout } from "../../lib/view";
import { Arc } from "../../components/Arc";
import { GlassCard } from "../../components/GlassCard";
import { SegmentMeter } from "../../components/SegmentMeter";
import { Slider } from "../../components/Slider";
import { Toggle } from "../../components/Toggle";
import { Button } from "../../components/Button";
import { CopyIcon, DoneIcon, FailedIcon, RefreshIcon, ShieldIcon } from "../../components/Icons";
import { MetricLabel } from "../../components/MetricLabel";
import { PasswordReference } from "./PasswordReference";
import "./password.css";
import "./password-reference.css";

/** Сколько держится ответ кнопки копирования. */
const COPIED_FOR = 1500;

const DEFAULTS: Options = { length: 16, upper: true, digits: true, symbols: true };

/** Порядок разрядов в счётчике — тот же, что в переключателях под ним. */
const CLASSES: CharClass[] = ["lower", "upper", "digit", "symbol"];
const CLASS_LABEL: Record<CharClass, string> = {
  lower: "a",
  upper: "A",
  digit: "0",
  symbol: "#",
};

export function PasswordWidget() {
  useWindowDrag();
  const t = useText();
  const reference = useLook().effectiveDesign === "reference";
  const layout = useWidgetLayout();
  const referenceNow = useRef(reference);
  referenceNow.current = reference;
  const [options, setOptions] = useState<Options>(DEFAULTS);
  // Первый пароль готов уже к первой отрисовке: пустое поле мелькнуло бы
  const [value, setValue] = useState(() => generate(DEFAULTS));
  const [copied, setCopied] = useState<boolean | undefined>(undefined);
  const resultRevision = useRef(0);
  const copyRequest = useRef(0);

  // Пароль перегенерируется на каждое изменение настроек: иначе на экране
  // висел бы пароль, не отвечающий тому, что показывают переключатели
  useEffect(() => {
    resultRevision.current++;
    setValue(generate(options));
    setCopied(undefined);
  }, [options]);

  const change = useCallback(<K extends keyof Options>(key: K, next: Options[K]) => {
    setOptions((current) => ({ ...current, [key]: next }));
  }, []);

  const timer = useRef<number | undefined>(undefined);
  useEffect(() => () => window.clearTimeout(timer.current), []);

  const copy = () => {
    const revision = resultRevision.current;
    const request = ++copyRequest.current;
    void copyToClipboard(value).then((done) => {
      // В GPT старый ответ копии не подтверждает уже созданный новый пароль.
      // Claude сохраняет прежнюю обработку ответа.
      if (referenceNow.current && (revision !== resultRevision.current || request !== copyRequest.current)) return;
      setCopied(done);
      window.clearTimeout(timer.current);
      timer.current = window.setTimeout(() => setCopied(undefined), COPIED_FOR);
    });
  };

  const strength = strengthOf(options);
  const counts = useMemo(() => composition(value), [value]);

  if (reference) {
    return (
      <div className="widget widget--password-reference">
        <GlassCard accent="neutral" flush>
          <PasswordReference
            layout={layout}
            options={options}
            value={value}
            strength={strength}
            counts={counts}
            copied={copied}
            onChange={change}
            onCopy={copy}
            onGenerate={() => {
              resultRevision.current++;
              window.clearTimeout(timer.current);
              setCopied(undefined);
              setValue(generate(options));
            }}
          />
        </GlassCard>
      </div>
    );
  }

  return (
    <div className="widget">
      <GlassCard accent="neutral" flush>
        <div className="pass">
          <div className="pass__row">
            {/*
              Пароль выделяется мышью: копирование кнопкой не единственный путь.
              Поэтому же он помечен как не-ручка — иначе окно уезжало бы за
              курсором вместо выделения.
            */}
            <output className="pass__value" data-no-drag>
              {[...value].map((character, index) => (
                <span key={index} data-class={classOf(character)}>
                  {character}
                </span>
              ))}
            </output>
            <Button
              variant="outline"
              icon={copied === undefined ? <CopyIcon /> : copied ? <DoneIcon /> : <FailedIcon />}
              label={
                copied === undefined ? t("common.copy") : copied ? t("common.copied") : t("common.failed")
              }
              onClick={copy}
            />
          </div>

          <div className="pass__row" data-accent={strength.accent}>
            {/*
              Стойкость в сжатой раскладке: то же число ступеней, свёрнутое в
              кольцо. Десять делений нарисованы штрихом на дорожке, заливка
              идёт по ним, а цвет — тот же, что у линейной шкалы рядом.
              Показывается ровно одна из двух: порог гасит вторую.
            */}
            <span className="pass__ring" title={t(strength.label)}>
              <Arc value={strength.level / STRENGTH_STEPS} />
            </span>
            <SegmentMeter
              level={strength.level}
              total={STRENGTH_STEPS}
              accent={strength.accent}
            />
            <span className="pass__counts">
              {CLASSES.filter((kind) => counts[kind] > 0).map((kind) => (
                <span key={kind} className="pass__count" data-class={kind}>
                  <span className="pass__class">{CLASS_LABEL[kind]}</span>
                  <span className="tabular">{counts[kind]}</span>
                </span>
              ))}
            </span>
          </div>

          <div className="pass__row">
            <span className="pass__caption">{t("password.length")}</span>
            <Slider
              value={options.length}
              min={MIN_LENGTH}
              max={MAX_LENGTH}
              onChange={(next) => change("length", next)}
            />
            <span className="pass__length tabular">{options.length}</span>
          </div>

          <div className="pass__row pass__row--controls">
            <Toggle label="A-Z" active={options.upper} onChange={(next) => change("upper", next)} />
            <Toggle
              label="0-9"
              active={options.digits}
              onChange={(next) => change("digits", next)}
            />
            <Toggle
              label="!@#"
              active={options.symbols}
              onChange={(next) => change("symbols", next)}
            />
            <span className="pass__strength" data-accent={strength.accent}>
              <MetricLabel icon={<ShieldIcon />}>{t(strength.label)}</MetricLabel>
            </span>
            <Button icon={<RefreshIcon />} onClick={() => setValue(generate(options))}>
              {t("password.generate")}
            </Button>
          </div>
        </div>
      </GlassCard>
    </div>
  );
}
