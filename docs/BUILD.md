# Сборка и проверка Papka

## Выпуск: GitHub Actions, только вручную

Workflow — `.github/workflows/windows-portable.yml`, запускается только
вручную (`workflow_dispatch`): коммиты и pull request сборку не вызывают.

1. GitHub → Aev199/papka → Actions → «Windows portable EXE (manual)».
2. Run workflow, ветка `feat/native-gpui-foundation`, поля — см.
   [UPDATES.md](UPDATES.md).
3. Сборка проверяет компиляцию, запускает тесты ядра, CLI и интерфейса,
   собирает `Papka.exe`, пробует снять скриншот окна и публикует выпуск.

Красная сборка: открыть упавший шаг и прочитать лог. EXE не подписан,
Windows при первом запуске показывает предупреждение SmartScreen.

## Локальные проверки (Linux или Windows)

```
cargo test --workspace
cargo check --workspace --all-targets --target x86_64-pc-windows-gnu
rustc --edition=2024 --test crates/desktop/src/layout.rs -o /tmp/lt && /tmp/lt
rustc --edition=2024 --test crates/desktop/src/palette.rs -o /tmp/pt && /tmp/pt
```

На Linux для этого нужны пакеты `libxkbcommon-dev`, `libxkbcommon-x11-dev`
(тесты интерфейса) и `mingw-w64` с целью `x86_64-pc-windows-gnu`
(Windows-проверка). Для запуска окна в Xvfb (`tools/ui-smoke`) —
`xvfb`, `openbox`, `xcompmgr`, `xdotool`, `x11-utils`, `imagemagick` и
программный Vulkan `mesa-vulkan-drivers`.

На Windows: Rust stable и MSVC Build Tools, затем
`cargo run -p filemanager-desktop --release`.

## Что на Linux не проверить

Перетаскивание с Проводником, контекстное меню и миниатюры Windows
Shell, предпросмотр Office/PDF, Корзину, диски и обновление программы.
Это проверяется только на собранном EXE по [SMOKE_TEST.md](SMOKE_TEST.md).
