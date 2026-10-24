// Собирает сведения о чужом коде, который едет в приложении или им используется.
//
// Два выхода из одних и тех же данных, чтобы они не разошлись:
// - THIRD-PARTY-NOTICES.txt — полный текст: каждый компонент с версией,
//   лицензией и источником, и все тексты лицензий. Едет в установщике рядом с
//   программой и открывается из раздела «Лицензия»;
// - src/lib/credits.json — выжимка для самого раздела: сводка поставки (три
//   части, число компонентов, перечень лицензий) и поимённо скачиваемые
//   программы и сервисы.
//
// Лицензии берутся из манифестов, а не из памяти: у крейтов — `cargo metadata`,
// у пакетов npm — их package.json, у сборок сайдкара — .nuspec из кеша NuGet.
// Тексты — из файлов самих пакетов; эталонные (MPL, LGPL, Apache, загрузчик
// WebView2) лежат в `licenses/`.
//
// Запуск: `npm run notices`. Тест `notices::tests::notices_are_fresh` сверяет
// отпечатки входных файлов и падает, если список устарел.

import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, readdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const NUGET = join(homedir(), ".nuget", "packages");
const TARGET = "x86_64-pc-windows-msvc";

/** Файлы, чьи отпечатки определяют состав: поменялся любой — список устарел. */
const INPUTS = ["src-tauri/Cargo.lock", "package-lock.json", "src-sensors/Sensors.csproj"];

const read = (path) => readFileSync(path, "utf8").replace(/\r\n/g, "\n");
const standard = (name) => read(join(ROOT, "licenses", name)).trimEnd();

/** Отпечаток без оглядки на переводы строк: git меняет их при выгрузке. */
function fingerprint(path) {
  return createHash("sha256").update(read(join(ROOT, path))).digest("hex");
}

// ---------------------------------------------------------------- тексты

const MIT_BODY = `Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.`;

const mit = (holder) => `MIT License\n\nCopyright (c) ${holder}\n\n${MIT_BODY}`;

const bsd3 = (holder) => `BSD 3-Clause License

Copyright (c) ${holder}

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions are met:

1. Redistributions of source code must retain the above copyright notice, this
   list of conditions and the following disclaimer.

2. Redistributions in binary form must reproduce the above copyright notice,
   this list of conditions and the following disclaimer in the documentation
   and/or other materials provided with the distribution.

3. Neither the name of the copyright holder nor the names of its
   contributors may be used to endorse or promote products derived from
   this software without specific prior written permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS"
AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE
FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL
DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER
CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY,
OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.`;

/**
 * Тексты лицензий без повторов: одинаковый текст печатается один раз, а у
 * компонента стоит его номер. Иначе двести копий Apache-2.0 сделали бы файл
 * в три мегабайта.
 */
const texts = new Map();
function cite(text) {
  const body = text.replace(/\r\n/g, "\n").replace(/[ \t]+$/gm, "").trim();
  const key = createHash("sha256").update(body).digest("hex");
  if (!texts.has(key)) texts.set(key, { id: texts.size + 1, body });
  return texts.get(key).id;
}

/** Файлы лицензий и уведомлений, которые пакет везёт с собой. */
function licenseFiles(dir, extra = []) {
  const pattern = /^(licen[cs]e|copying|notice|copyright|unlicense|third[-_]party)/i;
  const names = readdirSync(dir).filter(
    (name) => pattern.test(name) && !/\.spdx$/i.test(name) && statSync(join(dir, name)).isFile(),
  );
  for (const name of extra) if (name && !names.includes(name)) names.push(name);
  return names.sort();
}

// ---------------------------------------------------------------- Rust

function crates() {
  const raw = execFileSync(
    "cargo",
    ["metadata", "--format-version", "1", "--filter-platform", TARGET, "--locked", "--manifest-path", join(ROOT, "src-tauri", "Cargo.toml")],
    { encoding: "utf8", maxBuffer: 256 * 1024 * 1024 },
  );
  const meta = JSON.parse(raw);
  const packages = new Map(meta.packages.map((p) => [p.id, p]));
  const nodes = new Map(meta.resolve.nodes.map((n) => [n.id, n]));
  const root = meta.resolve.root;

  // Только обычные зависимости: сборочные и тестовые в программу не попадают
  const normal = (node) => node.deps.filter((d) => d.dep_kinds.some((k) => k.kind === null)).map((d) => d.pkg);

  const shipped = new Set();
  const stack = [root];
  while (stack.length > 0) {
    const id = stack.pop();
    if (shipped.has(id)) continue;
    shipped.add(id);
    stack.push(...normal(nodes.get(id)));
  }
  shipped.delete(root);

  const direct = new Set(normal(nodes.get(root)));
  return [...shipped]
    .map((id) => packages.get(id))
    .sort((a, b) => a.name.localeCompare(b.name) || a.version.localeCompare(b.version))
    .map((p) => ({ ...p, direct: direct.has(p.id) }));
}

/**
 * Файлы лицензий из репозиториев тех крейтов, что в пакете их не везут.
 * Взяты из самих репозиториев 29 сентября 2026: строка авторства там точнее,
 * чем имена из поля `authors` манифеста (у alloc-stdlib правообладатель —
 * Dropbox, а не автор пакета). Нет тут репозитория — шаблон по SPDX.
 */
const UPSTREAM = {
  "https://github.com/wravery/webview2-rs": ["webview2-rs.txt"],
  "https://github.com/djc/tokio-imap": ["tokio-imap-MIT.txt"],
  "https://github.com/open-i18n/rust-unic": ["rust-unic-MIT.txt"],
  "https://github.com/dropbox/rust-alloc-no-stdlib": ["rust-alloc-no-stdlib.txt"],
};

/**
 * Лицензии частей пакета, лежащие не в корне. Корневой поиск их не видит, а
 * требуют они своего текста в уведомлениях (проверка лицензий 15 октября).
 */
const NESTED = {
  "regex-syntax": ["src/unicode_tables/LICENSE-UNICODE"],
};

/**
 * Ветка «A OR B», текст которой пакет везёт сам. Общее правило выбирает по
 * списку `PERMISSIVE`, а в уведомление кладутся файлы пакета — и у этих двух
 * подпись разошлась бы с текстом: у dunce файл CC0, у brotli-decompressor —
 * BSD-3-Clause (проверка лицензий 15 октября).
 */
const ELECTED = {
  dunce: "CC0-1.0",
  "brotli-decompressor": "BSD-3-Clause",
};

/**
 * Правообладатель для шаблона MIT — из заголовков исходников, а не из поля
 * `authors`: там «Stalwart Labs» без «LLC» и без года.
 */
const HOLDERS = {
  "mail-parser": "2020 Stalwart Labs LLC <hello@stalw.art>",
  hashify: "2025 Stalwart Labs LLC <hello@stalw.art>",
};

const normalRepo = (url) => (url ?? "").replace(/\.git$/, "").replace(/\/+$/, "");

/** Лицензия крейта: его файлы, файлы его репозитория или стандартный текст по SPDX. */
function crateTexts(p) {
  const dir = dirname(p.manifest_path);
  const expression = p.license ?? "";
  const choice = ELECTED[p.name] ?? elected(expression);
  let files = licenseFiles(dir, p.license_file ? [p.license_file] : []);

  // «Apache-2.0 OR GPL-2.0-only» и подобные: берём разрешительную ветку, и
  // текст GPL, который пакет везёт на выбор, в уведомления не кладём. Узнаём
  // его по содержимому, а не по имени: у проектов GNU он зовётся COPYING
  if (PERMISSIVE.includes(choice) && /GPL/.test(expression)) {
    files = files.filter((name) => !/GNU (LESSER |AFFERO )?GENERAL PUBLIC LICENSE/.test(read(join(dir, name))));
  }

  // Лицензии частей, которые лежат не в корне пакета: у regex-syntax таблицы
  // Unicode идут под своим соглашением Unicode (DFS-2016), и корневые MIT и
  // Apache его не заменяют
  for (const nested of NESTED[p.name] ?? []) {
    if (existsSync(join(dir, nested))) files.push(nested);
  }

  if (files.length > 0) return { ids: files.map((name) => cite(read(join(dir, name)))), note: "" };

  const upstream = UPSTREAM[normalRepo(p.repository)];
  if (upstream) {
    return {
      ids: upstream.map((name) => cite(read(join(ROOT, "licenses", "crates", name)))),
      note: " (from the upstream repository; the package ships no license file)",
    };
  }

  // Шаблон — ровно для выбранной ветки: при «MPL-2.0 OR BSD-3-Clause» в
  // уведомлении обязан стоять текст BSD, под которой пакет и используется.
  // AND и WITH (исключения вроде LLVM) шаблоном не описать — только руками
  const synthetic = " (standard text; the package ships no license file)";
  const holder = HOLDERS[p.name] ?? (p.authors?.length ? p.authors.join(", ") : `the ${p.name} authors`);
  const templates = {
    MIT: () => mit(holder),
    "Apache-2.0": () => standard("Apache-2.0.txt"),
    "BSD-3-Clause": () => bsd3(holder),
    "MPL-2.0": () => standard("MPL-2.0.txt"),
  };
  const template = templates[choice];
  if (template) return { ids: [cite(template())], note: synthetic };
  throw new Error(`${p.name} ${p.version}: нет файла лицензии и нет шаблона для «${expression}» — нужен разбор руками`);
}

/**
 * Есть ли в выражении SPDX именно эта лицензия. Не `\b`: у него «MIT» находится
 * и внутри «MIT-0», а это другая лицензия.
 */
function has(expression, id) {
  const escaped = id.replace(/[.]/g, "\\.");
  return new RegExp(`(?<![\\w.-])${escaped}(?![\\w.-])`).test(expression);
}

/** Разрешительные лицензии в порядке предпочтения, когда выражение даёт выбор. */
const PERMISSIVE = ["MIT", "Apache-2.0", "BSD-3-Clause", "BSD-2-Clause", "ISC", "0BSD", "Zlib", "BSL-1.0", "Unlicense"];

/**
 * Что выбрано из «A OR B». Выражение с AND остаётся целым — там обязательны
 * обе лицензии; с WITH тоже — исключение из лицензии не отбросить.
 */
function elected(expression) {
  if (/\bAND\b|\bWITH\b/.test(expression) || !/\bOR\b|\//.test(expression)) return expression;
  return PERMISSIVE.find((id) => has(expression, id)) ?? expression;
}

// ---------------------------------------------------------------- npm

function npmPackages() {
  const manifest = JSON.parse(read(join(ROOT, "package.json")));
  const seen = new Map();
  const walk = (name) => {
    if (seen.has(name)) return;
    const dir = join(ROOT, "node_modules", ...name.split("/"));
    const pkg = JSON.parse(read(join(dir, "package.json")));
    seen.set(name, { name, version: pkg.version, license: pkg.license, dir, url: repoUrl(pkg) });
    for (const dep of Object.keys(pkg.dependencies ?? {})) walk(dep);
  };
  for (const dep of Object.keys(manifest.dependencies ?? {})) walk(dep);
  return [...seen.values()].sort((a, b) => a.name.localeCompare(b.name));
}

function repoUrl(pkg) {
  const repo = typeof pkg.repository === "string" ? pkg.repository : pkg.repository?.url;
  if (!repo) return pkg.homepage ?? "";
  return repo.replace(/^git\+/, "").replace(/\.git$/, "").replace(/^git:\/\//, "https://");
}

/**
 * Служебный код сборщика, который Vite и Rolldown кладут в сам бандл:
 * предзагрузка чанков ленивого реестра и рантайм модулей. Своих зависимостей
 * они в бандл не несут, поэтому берём только их собственную лицензию.
 */
function bundlerRuntime() {
  const vite = JSON.parse(read(join(ROOT, "node_modules", "vite", "package.json")));
  const rolldown = JSON.parse(read(join(ROOT, "node_modules", "rolldown", "package.json")));
  const viteLicense = read(join(ROOT, "node_modules", "vite", "LICENSE.md"))
    .split("# Licenses of bundled dependencies")[0]
    .replace("# Vite core license", "")
    .trim();
  return [
    {
      name: "vite (runtime helpers)",
      version: vite.version,
      license: vite.license,
      url: repoUrl(vite),
      ids: [cite(viteLicense)],
    },
    {
      name: "rolldown (runtime helpers)",
      version: rolldown.version,
      license: rolldown.license,
      url: repoUrl(rolldown),
      ids: [cite(read(join(ROOT, "node_modules", "rolldown", "LICENSE")))],
    },
  ];
}

// ---------------------------------------------------------------- сайдкар

function nuspec(id, version) {
  const dir = join(NUGET, id.toLowerCase(), version);
  const xml = read(join(dir, `${id.toLowerCase()}.nuspec`));
  const tag = (name) => xml.match(new RegExp(`<${name}[^>]*>([^<]*)</${name}>`))?.[1]?.trim() ?? "";
  const license = xml.match(/<license type="(expression|file)">([^<]*)<\/license>/);
  const repository = xml.match(/<repository[^>]*url="([^"]*)"(?:[^>]*commit="([^"]*)")?/);
  return {
    dir,
    licenseType: license?.[1] ?? "",
    license: license?.[2] ?? "",
    copyright: tag("copyright"),
    project: tag("projectUrl"),
    repo: repository?.[1]?.replace(/^git:\/\//, "https://").replace(/\.git$/, "") ?? "",
    commit: repository?.[2] ?? "",
  };
}

/**
 * Сборки, вшитые в windgets-sensors.exe: всё, что NuGet разложил для рантайма win-x64.
 * Сборка с пустым рантаймом (System.Management) — ссылка на .NET Framework, и в
 * exe она не едет.
 */
function sidecar() {
  const assets = JSON.parse(read(join(ROOT, "src-sensors", "obj", "project.assets.json")));
  const target = assets.targets[".NETFramework,Version=v4.7.2/win-x64"];
  const items = [];

  for (const [key, info] of Object.entries(target)) {
    const runtime = Object.keys(info.runtime ?? {}).filter((path) => !path.endsWith("_._"));
    if (info.type !== "package" || runtime.length === 0) continue;

    const [id, version] = key.split("/");
    const spec = nuspec(id, version);
    const ids = [];

    if (spec.licenseType === "file") {
      ids.push(cite(read(join(spec.dir, spec.license))));
    } else if (spec.license === "MPL-2.0") {
      ids.push(cite(standard("MPL-2.0.txt")));
    } else if (spec.license === "MIT" && spec.copyright.includes("Microsoft")) {
      // Лицензия репозиториев dotnet: MIT на имя .NET Foundation
      ids.push(cite(mit(".NET Foundation and Contributors")));
    } else if (spec.license === "MIT") {
      ids.push(cite(mit(spec.copyright || id)));
    } else {
      throw new Error(`${id} ${version}: нет шаблона для «${spec.license}»`);
    }
    for (const name of licenseFiles(spec.dir)) if (/notice|third/i.test(name)) ids.push(cite(read(join(spec.dir, name))));

    const source = spec.commit ? `${spec.repo} (commit ${spec.commit})` : spec.repo || spec.project;
    items.push({
      name: id,
      version,
      license: spec.licenseType === "file" ? "Apache-2.0" : spec.license,
      url: spec.repo || spec.project,
      source,
      ids,
    });
  }

  if (items.some((item) => item.license === "Apache-2.0" && item.name !== "HidSharp")) {
    throw new Error("лицензия-файл не у HidSharp: проверьте, что она Apache-2.0, и поправьте генератор");
  }

  // Вшитое внутри самих сборок — объявлено их авторами
  items.push(
    {
      name: "PawnIO.Modules (embedded in LibreHardwareMonitorLib)",
      // LHM 0.9.6 вшивает выпуск 0.2.2 (LibreHardwareMonitor#2174). Коммит
      // обязателен: без него архив исходников к выпуску брал бы `HEAD`, а
      // LGPL просит исходники именно той версии, что поставляется
      version: "0.2.2 (as shipped in LibreHardwareMonitorLib 0.9.6)",
      license: "LGPL-2.1",
      url: "https://github.com/namazso/PawnIO.Modules",
      source: "https://github.com/namazso/PawnIO.Modules (commit e12a858d952461ee2e919897cacbff7f905fe370)",
      ids: [cite(standard("LGPL-2.1.txt"))],
    },
    {
      name: "PCI ID Repository — pci.ids (embedded in DiskInfoToolkit)",
      family: "BSD-3-Clause",
      version: "2025.11.16",
      license: "BSD-3-Clause (elected from GPL-2.0-or-later OR BSD-3-Clause)",
      url: "https://pci-ids.ucw.cz/",
      source: "https://pci-ids.ucw.cz/",
      ids: [cite(bsd3("Martin Mares and Albert Pool"))],
    },
    {
      name: "USB ID Repository — usb.ids (embedded in DiskInfoToolkit)",
      family: "BSD-3-Clause",
      version: "2025.09.15",
      license: "BSD-3-Clause (elected from GPL-2.0-or-later OR BSD-3-Clause)",
      url: "http://www.linux-usb.org/usb-ids.html",
      source: "http://www.linux-usb.org/usb.ids",
      ids: [cite(bsd3("Stephen J. Gowdy <linux.usb.ids@gmail.com>"))],
    },
    {
      name: "CrystalDiskInfo (code DiskInfoToolkit is based on)",
      version: "",
      license: "MIT",
      url: "https://github.com/hiyohiyo/CrystalDiskInfo",
      source: "https://github.com/hiyohiyo/CrystalDiskInfo",
      ids: [cite(mit("2008-2023 hiyohiyo"))],
    },
  );
  return items;
}

// ---------------------------------------------------------------- не в поставке

/** Скачиваются по согласию человека с выпусков авторов и работают отдельными программами. */
const PROGRAMS = [
  {
    id: "ffmpeg",
    name: "FFmpeg",
    license: "GPL-3.0 (gyan.dev essentials build)",
    brief: { license: "GPL-3.0" },
    url: "https://ffmpeg.org/",
    note: "Essentials build by gyan.dev, downloaded from https://github.com/GyanD/codexffmpeg/releases. FFmpeg source code: https://ffmpeg.org/download.html. A copy already installed on the system is used as is.",
  },
  {
    id: "ytdlp",
    name: "yt-dlp",
    license: "GPL-3.0-or-later (PyInstaller build; yt-dlp's own code is Unlicense)",
    brief: { license: "GPL-3.0+" },
    url: "https://github.com/yt-dlp/yt-dlp",
    note: "Official yt-dlp.exe from https://github.com/yt-dlp/yt-dlp/releases. yt-dlp's own code is released under the Unlicense; the PyInstaller-bundled executable includes GPLv3+ code and is distributed under GPLv3+, see its THIRD_PARTY_LICENSES.txt.",
  },
  { id: "quickjs", name: "QuickJS-NG", license: "MIT", url: "https://github.com/quickjs-ng/quickjs", note: "Official qjs build from https://github.com/quickjs-ng/quickjs/releases, used by yt-dlp as its JavaScript runtime." },
  { id: "presentmon", name: "PresentMon", license: "MIT", url: "https://github.com/GameTechDev/PresentMon", note: "Official console build from https://github.com/GameTechDev/PresentMon/releases, used by the frame counter to read frame events from ETW." },
  {
    id: "pawnio",
    name: "PawnIO",
    license: "Official binary: proprietary, redistributable (source: GPL-2.0-or-later with the PawnIO exception)",
    brief: { license: "Proprietary" },
    url: "https://github.com/namazso/PawnIO",
    note: "Kernel driver by namazso, installed only on the user's request by its official installer from https://github.com/namazso/PawnIO.Setup/releases and left unchanged. The author licenses the official binary editions as proprietary with redistribution allowed; the published source code is GPL-2.0-or-later with the PawnIO exception. LibreHardwareMonitor talks to the driver solely through the device IO control interface. Source code: https://github.com/namazso/PawnIO",
  },
];

const SERVICES = [
  {
    id: "openmeteo",
    name: "Open-Meteo",
    license: "CC BY 4.0 (weather data)",
    brief: { license: "CC BY 4.0" },
    url: "https://open-meteo.com/",
    note: "Weather data by Open-Meteo.com, licensed under CC BY 4.0. City search location data based on GeoNames (https://www.geonames.org/), licensed under CC BY 4.0. Terms: https://open-meteo.com/en/terms",
  },
  {
    id: "mlab",
    name: "M-Lab (NDT7 speed test)",
    license: "Open data",
    brief: { name: "M-Lab", license: "" },
    url: "https://www.measurementlab.net/",
    note: "Speed test results, including the IP address, date and time, are published by M-Lab as open data. Privacy policy: https://www.measurementlab.net/privacy/",
  },
  {
    id: "spotify",
    name: "Spotify Web API",
    license: "Spotify Developer Terms",
    brief: { license: "" },
    url: "https://developer.spotify.com/terms",
    note: "Spotify is a trademark of Spotify AB. Windgets is not affiliated with, sponsored or endorsed by Spotify. The widget uses the Web API with your own Client ID.",
  },
];

// ---------------------------------------------------------------- выжимка

/**
 * Часть поставки для раздела «Лицензия»: заглавный компонент и сколько всего.
 * Заглавный ищется по имени пакета — пропади он из зависимостей, сводка не
 * соберётся, а не назовёт то, чего в поставке уже нет.
 */
function part(id, pkg, lead, items) {
  if (!items.some((item) => item.name === pkg)) throw new Error(`${id}: в составе нет «${pkg}»`);
  return { id, lead, count: items.length };
}

/**
 * Лицензии поставки одним рядом, самые частые первыми.
 *
 * Считается по той ветке, которой пользуемся (`elected`): у «MIT OR
 * Apache-2.0» в ряд идёт MIT. Выражение с `AND` даёт все свои лицензии,
 * исключение после `WITH` лицензией не считается. `family` — для компонента,
 * чья лицензия не записана выражением SPDX.
 */
function families(items) {
  const count = new Map();
  for (const item of items) {
    // Выбор ветки уже сделан (`ELECTED`) — пересчитывать его по исходному
    // выражению значило бы потерять, например, CC0-1.0 у dunce
    const expression = item.family ?? item.elected ?? elected(item.license);
    for (const id of expression.replace(/\s+WITH\s+\S+/g, "").split(/[\s()/]+/)) {
      if (id === "" || id === "AND" || id === "OR") continue;
      if (!/^[A-Za-z0-9][A-Za-z0-9.+-]*$/.test(id)) throw new Error(`${item.name}: «${id}» — не лицензия SPDX`);
      count.set(id, (count.get(id) ?? 0) + 1);
    }
  }
  return [...count].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0])).map(([id]) => id);
}

// ---------------------------------------------------------------- сборка

function main() {
  const inputs = Object.fromEntries(INPUTS.map((path) => [path, fingerprint(path)]));

  const rust = crates().map((p) => {
    const { ids, note } = crateTexts(p);
    return {
      name: p.name,
      version: p.version,
      license: p.license ?? "",
      elected: ELECTED[p.name] ?? elected(p.license ?? ""),
      url: p.repository ?? `https://crates.io/crates/${p.name}`,
      direct: p.direct,
      note,
      ids,
    };
  });
  const webview2 = {
    name: "Microsoft WebView2 SDK loader (WebView2LoaderStatic.lib, via webview2-com-sys)",
    // Текст — BSD-3-Clause слово в слово, с Microsoft в третьем пункте
    family: "BSD-3-Clause",
    version: "",
    license: "BSD-3-Clause-style Microsoft license",
    url: "https://www.nuget.org/packages/Microsoft.Web.WebView2",
    ids: [cite(standard("WebView2-SDK.txt"))],
  };
  const web = [
    ...npmPackages().map((p) => ({ ...p, ids: licenseFiles(p.dir).map((name) => cite(read(join(p.dir, name)))) })),
    ...bundlerRuntime(),
  ];
  for (const p of web) if (p.ids.length === 0) throw new Error(`${p.name}: пакет без файла лицензии`);
  const dotnet = sidecar();

  const line = (item, extra = "") =>
    `  - ${item.name}${item.version ? ` ${item.version}` : ""}\n` +
    `    License: ${item.license}${item.elected && item.elected !== item.license ? ` (used under ${item.elected})` : ""}${extra}\n` +
    `    Source: ${item.source ?? item.url}\n` +
    (item.ids ? `    License text: ${item.ids.map((id) => `[${id}]`).join(", ")}${item.note ?? ""}\n` : "");

  const out = [];
  out.push("THIRD-PARTY SOFTWARE NOTICES AND INFORMATION");
  out.push("Windgets");
  out.push("");
  out.push("Windgets is licensed under the MIT License (see LICENSE.txt). It includes and uses");
  out.push("the third-party components listed below, each under its own license. License texts");
  out.push("are reproduced in Part 5; each component refers to them by number.");
  out.push("");
  out.push("Generated by scripts/notices.mjs from the package manifests. Do not edit by hand.");
  out.push(`Inputs: ${Object.entries(inputs).map(([path, hash]) => `${path} sha256=${hash}`).join("; ")}`);
  out.push("");
  out.push("=".repeat(79));
  out.push("PART 1. Hardware sensors: windgets-sensors.exe (.NET, assemblies embedded in the executable)");
  out.push("=".repeat(79));
  out.push("");
  out.push("The source code of the MPL-2.0 and LGPL-2.1 components below is available at the");
  out.push("listed addresses. They are used unmodified.");
  out.push("");
  for (const item of dotnet) out.push(line(item));
  out.push("=".repeat(79));
  out.push(`PART 2. Rust crates compiled into windgets.exe (${rust.length} crates)`);
  out.push("=".repeat(79));
  out.push("");
  out.push("MPL-2.0 crates are used unmodified; their source code is available on crates.io.");
  out.push("");
  for (const item of rust) out.push(line(item));
  out.push(line(webview2));
  out.push("=".repeat(79));
  out.push("PART 3. JavaScript libraries bundled into the user interface");
  out.push("=".repeat(79));
  out.push("");
  for (const item of web) out.push(line(item));
  out.push("=".repeat(79));
  out.push("PART 4. Not included: programs downloaded on request, and online services");
  out.push("=".repeat(79));
  out.push("");
  out.push("These programs are NOT included in Windgets. The application downloads them from");
  out.push("the authors' official releases only when you ask it to, verifies their SHA-256, and");
  out.push("runs them as separate programs without modifying them. Their own licenses apply.");
  out.push("");
  for (const item of PROGRAMS) out.push(`  - ${item.name}\n    License: ${item.license}\n    ${item.note}\n`);
  out.push("Online services used by the widgets:");
  out.push("");
  for (const item of SERVICES) out.push(`  - ${item.name}\n    ${item.note}\n`);
  out.push("=".repeat(79));
  out.push("PART 5. License texts");
  out.push("=".repeat(79));
  for (const { id, body } of texts.values()) {
    out.push("");
    out.push("-".repeat(79));
    out.push(`[${id}]`);
    out.push("-".repeat(79));
    out.push(body);
  }
  out.push("");
  writeFileSync(join(ROOT, "THIRD-PARTY-NOTICES.txt"), out.join("\n"), "utf8");

  // Выжимка для раздела «Лицензия». Поставка — сводкой: три части, сколько в
  // каждой чужого кода и под какими лицензиями он весь. Поимённо её состав
  // живёт только в полном списке: полсотни строк в интерфейсе никто не читал
  // (этап 52, слово пользователя), а закон требует текстов, и они в файле.
  // Скачиваемые программы и сервисы — поимённо: их всего шесть, и о каждом
  // есть что сказать
  const brief = (item) => ({
    ...(item.id ? { id: item.id } : {}),
    name: item.brief?.name ?? item.name,
    license: item.brief?.license ?? item.license,
    url: item.url,
  });
  const credits = {
    inputs,
    parts: [
      part("sidecar", "LibreHardwareMonitorLib", "LibreHardwareMonitor", dotnet),
      part("rust", "tauri", "Tauri", [...rust, webview2]),
      part("web", "react", "React", web),
    ],
    licenses: families([...dotnet, ...rust, webview2, ...web]),
    programs: PROGRAMS.map(brief),
    services: SERVICES.map(brief),
  };
  writeFileSync(join(ROOT, "src", "lib", "credits.json"), `${JSON.stringify(credits, null, 2)}\n`, "utf8");

  console.log(`crates: ${rust.length}, web: ${web.length}, sidecar: ${dotnet.length}, license texts: ${texts.size}`);
}

main();
