<#
    Запуск приложения в режиме разработки одной командой.

    Приложение просит права администратора манифестом, и `npm run tauri dev`
    из обычной консоли не поднимается вовсе — дочерний процесс отвечает
    «требуется повышение» (os error 740). Поэтому скрипт сперва смотрит, где
    он сам, и при надобности перезапускает себя повышенным.

    Релиз при этом не собирается: `tauri dev` поднимает vite на 1420 и
    подключает к нему отладочную сборку. Первый заход собирает Rust целиком,
    дальше пересборка идёт по изменённому.

    ВАЖНО: файл обязан лежать в UTF-8 С BOM. PowerShell 5.1 читает .ps1 без
    BOM как ANSI, русские комментарии превращаются в кракозябры, и скрипт
    перестаёт разбираться вовсе — с ошибками, указывающими не туда, где
    ошибка. Выглядит как испорченный код, а не как испорченная кодировка.
#>

$ErrorActionPreference = 'Stop'

# Корень проекта — на уровень выше этого файла
$root = Split-Path -Parent $PSScriptRoot

# Уже запущенное приложение вторым экземпляром не поднимаем: два набора окон
# подерутся за сохранённые позиции
$running = Get-Process windgets -ErrorAction SilentlyContinue
if ($null -ne $running) {
    Write-Host "Приложение уже работает (pid $($running[0].Id))." -ForegroundColor Yellow
    Write-Host 'Выход — через пункт меню в трее.'
    exit 1
}

<#
    Порт 1420 у vite фиксированный (`strictPort`), и он же прописан в
    tauri.conf.json как devUrl. Забытый с прошлого раза `npm run dev` держит
    его и роняет запуск целиком, причём жалобой на vite, а не на порт.

    Сами не убиваем: чужой процесс на своей машине — решение хозяина, а не
    скрипта. Печатаем, кто держит, и чем его снять.
#>
$busy = Get-NetTCPConnection -LocalPort 1420 -State Listen -ErrorAction SilentlyContinue
if ($null -ne $busy) {
    $pid1420 = $busy[0].OwningProcess
    $owner = (Get-Process -Id $pid1420 -ErrorAction SilentlyContinue).ProcessName
    Write-Host "Порт 1420 занят: $owner (pid $pid1420)." -ForegroundColor Red
    Write-Host "Снять — Stop-Process -Id $pid1420 -Force, потом повторить запуск."
    exit 1
}

<#
    Повышение — последним, после проверок.

    Обе проверки выше обходятся без прав, и порядок тут не косметика: спроси
    мы UAC первым, диалог всплывал бы и на уже работающем приложении — то
    есть чаще всего, потому что чаще всего его запускают по второму разу.
#>
$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$elevated = (New-Object Security.Principal.WindowsPrincipal $identity).IsInRole(
    [Security.Principal.WindowsBuiltInRole]::Administrator
)

if (-not $elevated) {
    Write-Host 'Нужны права администратора — открываю повышенную консоль…' -ForegroundColor Yellow

    # -NoExit, чтобы окно осталось: в нём и живёт приложение, и там же виден
    # его вывод. Закрытие этой консоли завершает приложение
    Start-Process powershell -Verb RunAs -ArgumentList @(
        '-NoExit',
        '-NoProfile',
        '-ExecutionPolicy', 'Bypass',
        # Путь в кавычках: `Start-Process` склеивает массив пробелами и сам
        # ничего не обрамляет, поэтому путь с пробелом обрезался бы по первому
        '-File', "`"$PSCommandPath`""
    )
    exit 0
}

Set-Location $root

# Именно npm.cmd: политика выполнения сценариев запрещает npm.ps1, и в
# повышенной консоли это первое, обо что спотыкаешься
& npm.cmd run tauri dev
