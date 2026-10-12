// Первым: `tr!` виден модулям ниже по тексту без импорта
#[macro_use]
mod i18n;
mod autostart;
mod child;
mod clipboard;
mod converter;
mod disk;
mod downloader;
mod dpapi;
#[cfg(windows)]
mod file_drop;
mod journal;
mod lhm;
mod links;
mod mail;
mod media;
mod modules;
#[cfg(windows)]
mod native_frame;
mod notes;
mod notices;
mod output;
mod pawnio;
mod update;
mod picker;
mod ping;
mod poller;
mod processes;
mod sensors;
mod settings;
mod settings_window;
mod snap;
mod sound;
mod speedtest;
mod spotify;
mod support_reminder;
mod todo;
mod tools;
mod totp;
mod tray;
mod weather;
mod widget_menu;
mod widgets;
mod window_effects;
mod windows;

use tauri::Manager;

use crate::windows::Shutdown;

pub fn run() {
    let app = tauri::Builder::default()
        // Меню виджета по правой кнопке. Слушатель глобальный: событие меню
        // приходит всем сразу, и виджет опознаётся по идентификатору пункта
        .on_menu_event(widget_menu::handle)
        // Плагину доверяем только координаты и размер. Рамку и видимость он
        // тоже умеет восстанавливать, и однажды вернул бы виджету заголовок
        // окна из чужой записи.
        .plugin(
            tauri_plugin_window_state::Builder::default()
                // finish дисков сам восстанавливает позицию и ставит пресет.
                // Исключаем второе восстановление старого SIZE у дисков.
                .skip_initial_state("disks")
                .with_state_flags(
                    tauri_plugin_window_state::StateFlags::POSITION
                        | tauri_plugin_window_state::StateFlags::SIZE,
                )
                .build(),
        )
        .manage(poller::Telemetry::default())
        .manage(ping::PingState::default())
        .manage(sound::Sound::default())
        .manage(clipboard::Clipboard::default())
        .manage(processes::Processes::default())
        .manage(weather::Weather::default())
        .manage(converter::Converter::default())
        .manage(downloader::Downloader::default())
        .manage(Shutdown::default())
        .invoke_handler(tauri::generate_handler![
            poller::telemetry_state,
            ping::ping_state,
            sound::sound_state,
            sound::sound_control,
            sound::sound_icon,
            clipboard::clipboard_state,
            clipboard::clipboard_control,
            processes::processes_state,
            notes::notes_pages,
            notes::set_notes_pages,
            todo::todo_lists,
            todo::set_todo_lists,
            weather::weather_state,
            weather::weather_refresh,
            weather::weather_search,
            speedtest::speedtest_view,
            speedtest::speedtest_start,
            window_effects::appearance,
            window_effects::preview_appearance,
            window_effects::preview_widget_appearance,
            window_effects::preview_widget_scale_appearance,
            window_effects::commit_appearance,
            window_effects::set_palette,
            window_effects::set_glass,
            window_effects::set_accent_color,
            window_effects::set_custom_color,
            window_effects::reset_custom_colors,
            i18n::language,
            i18n::set_language,
            settings_window::open_widget_settings,
            settings_window::settings_view,
            settings_window::set_widget_option,
            settings_window::set_all_widget_option,
            settings_window::set_widget_cover,
            settings_window::set_widget_view,
            settings_window::set_widget_layout,
            settings_window::set_sound_mixer_collapsed,
            window_effects::set_design,
            settings_window::set_widget_size,
            settings_window::set_flag,
            settings_window::set_source,
            settings_window::set_weather_place,
            settings_window::set_disk_hidden,
            settings_window::set_metric_hidden,
            settings_window::set_sensor_pick,
            settings_window::reset_widget_places,
            settings_window::open_config_dir,
            settings_window::open_log_dir,
            support_reminder::support_reminder_answer,
            support_reminder::set_support_reminder_rare,
            settings_window::close_settings,
            modules::set_module,
            modules::set_all_modules,
            snap::begin_drag,
            snap::end_drag,
            windows::grow_window,
            windows::fit_window_height,
            windows::fit_sound_panel,
            widget_menu::open_widget_menu,
            widget_menu::open_task_menu,
            links::open_link,
            notices::open_notices,
            spotify::spotify_view,
            spotify::spotify_connect,
            spotify::spotify_disconnect,
            spotify::spotify_control,
            spotify::spotify_playlists,
            spotify::spotify_open,
            mail::mail_state,
            mail::mail_refresh,
            mail::mail_account,
            mail::set_mail_account,
            mail::mail_check,
            mail::forget_mail_account,
            mail::set_mail_preview,
            mail::mail_body,
            mail::mail_search,
            mail::mail_mark_read,
            mail::mail_mark_all_read,
            mail::mail_delete,
            totp::totp_state,
            totp::totp_codes,
            totp::add_totp_account,
            totp::rename_totp_account,
            totp::reorder_totp_accounts,
            totp::remove_totp_account,
            media::media_state,
            media::media_control,
            media::media_seek,
            media::pick_media_session,
            media::set_media_source,
            media::media_art,
            tools::tools_state,
            tools::tools_install,
            tools::tools_update,
            tools::tools_cancel,
            tools::tools_remove,
            pawnio::pawnio_state,
            pawnio::pawnio_install,
            pawnio::pawnio_cancel,
            pawnio::pawnio_remove,
            update::update_state,
            update::update_check,
            update::update_install,
            update::update_cancel,
            update::update_set_auto,
            output::pick_output_dir,
            output::reset_output_dir,
            output::open_output_dir,
            converter::converter_state,
            converter::converter_pick,
            converter::converter_remove,
            converter::converter_clear,
            converter::converter_set,
            converter::converter_start,
            converter::converter_cancel,
            converter::converter_reveal,
            converter::converter_frame,
            converter::converter_frame_hide,
            converter::converter_wave,
            converter::converter_listen,
            converter::converter_hush,
            downloader::downloader_state,
            downloader::downloader_start,
            downloader::downloader_options,
            downloader::downloader_cancel,
            downloader::downloader_reveal,
            downloader::downloader_forget,
        ])
        .setup(|app| {
            let handle = app.handle().clone();

            // Журнал — первым: и чтение настроек, и сайдкар, которого
            // поднимут ниже, пишут свои отказы в stderr (`journal.rs`)
            journal::start(&handle);

            let settings = settings::load(&handle);
            // Язык — до трея и окон: их подписи собираются на нём
            i18n::apply(settings.language.as_deref());
            app.manage(settings::SettingsState::new(settings));
            // Прилипание и блокировку оконная процедура читает из статики:
            // к настройкам из модального цикла перетаскивания не сходить
            settings_window::apply_flags(&handle);
            // История замеров лежит на диске: диаграмма должна быть с
            // прошлыми столбиками сразу, а не с первого замера после запуска
            app.manage(speedtest::Speedtest::new(speedtest::load_history(&handle)));
            app.manage(spotify::Session::new(&handle));
            app.manage(mail::Mail::new(mail::store::load(&handle)));
            // Фонового цикла у 2FA нет: коды считаются по запросу окна, а
            // будильник на смену кода ставит само окно — см. totp/mod.rs
            app.manage(totp::Totp::new(totp::store::load(&handle)));
            // Список увиденных источников с диска: раздел настроек показывает
            // его сразу, даже пока виджет спрятан и опроса нет
            app.manage(media::Media::new(media::store::load(&handle)));
            // Докачиваемые программы конвертера и загрузчика: что стоит и
            // когда проверяли — с диска, в сеть до нажатия кнопки не ходим
            app.manage(tools::Tools::new(&handle));
            // Драйвер PawnIO: ставили ли его мы — с диска; готов ли — по
            // устройству, на каждый вопрос (`pawnio.rs`)
            app.manage(pawnio::Pawnio::new(&handle));
            // Обновления Windgets: что нашла проверка — только в памяти, срок
            // недели — в `settings.json` (`update.rs`)
            app.manage(update::Updates::new());

            // Трей поднимаем до окон: если окно не откроется, управление
            // всё равно останется под рукой
            if let Err(error) = tray::install(&handle) {
                eprintln!("значок в трее не создался: {error}");
            }
            // Окна включённых модулей. Собираем здесь, в главном потоке до
            // событийного цикла; на ходу их собирает `modules::set` из пула
            windows::prepare_all(&handle);
            // Окно настроек собирается всегда: страницу виджета в нём
            // открывают изнутри webview, а оно само — не модуль
            settings_window::prepare(&handle);

            // Фоновые работы — только у включённых модулей и только после
            // окон: звук, процессы и прочие смотрят на видимость своего окна,
            // буферу окно служит владельцем, конвертеру — подложкой под
            // перетаскивание
            modules::boot(&handle);
            // Напоминание о донатах: свой поток, почти всё время спит. Пока
            // звать некуда (нет ни одной ссылки), не показывает ничего
            support_reminder::start(&handle);
            update::start(&handle);
            // Хвосты прерванных установок программ — всегда: место на диске
            // они занимают независимо от того, включён ли модуль
            tools::spawn(handle);
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("не удалось запустить приложение");

    app.run(|app, event| {
        if let tauri::RunEvent::ExitRequested { api, .. } = event {
            // Приложение живёт в трее: выключенный последний виджет — не повод
            // завершаться. Выход только через пункт меню, он ставит признак.
            if !app.state::<Shutdown>().started() {
                api.prevent_exit();
            }
        }
    });
}
