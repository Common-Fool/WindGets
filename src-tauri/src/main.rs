// В релизной сборке консольное окно рядом с виджетом не нужно
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    windgets_lib::run()
}
