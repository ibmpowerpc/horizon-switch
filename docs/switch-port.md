# Нативный порт Horizon на Switch

## Зафиксированная цель

- Upstream: https://github.com/horizon-rust/horizon
- Исследованный main: `c09ecc9b0cb965f22a6fd389daaa5db76bdddb25` (2026-11-15).
- Цель: первое семейство Nintendo Switch; приоритет — Switch Lite.
- Запуск: Atmosphère / hbmenu, `.nro`. Linux не является целевой средой.
- Форк: https://github.com/ibmpowerpc/horizon-switch ; Switch — основная цель main.
- Наработки порта интегрированы поверх upstream commit `41cdc1a8d2fd6dbcc8b56a831cf71f284f7973d3`.
- История и исходное дерево Horizon сохранены; локальный checkout разреженный для экономии диска.
- Подключение к существующим серверам Java Edition — обязательный критерий порта.
- Приоритет — offline-профили: локальный ник без Microsoft для серверов,
  допускающих такой вход. Microsoft-вход для `online-mode=true` — отдельный режим.
- Дополнительная условная цель — интерфейс в стиле Legacy Console Edition по
  примеру выбранного пользователем Legacy4J. Она не должна требовать модификаций Java-сервера
  или нарушать vanilla-подключение; при несовместимости интеграция исключается.

## Что установлено по исходникам

В [корневом Cargo.toml](https://github.com/horizon-rust/horizon/blob/c09ecc9b0cb965f22a6fd389daaa5db76bdddb25/Cargo.toml)
указаны Rust 1.95, edition 2024 и собственные патчи wgpu-hal/naga.
[horizon-client](https://github.com/horizon-rust/horizon/blob/c09ecc9b0cb965f22a6fd389daaa5db76bdddb25/crates/horizon-client/Cargo.toml)
зависит от winit 0.30.13, wgpu 30.0.1 и опционального gilrs для контроллеров.
[horizon-client-render](https://github.com/horizon-rust/horizon/blob/c09ecc9b0cb965f22a6fd389daaa5db76bdddb25/crates/horizon-client-render/Cargo.toml)
использует тот же графический стек и опциональный cpal 0.16 для звука.
Графические зависимости остаются и при отключении default features.

Это требует платформенной адаптации. Наличие ARM64 CPU и Rust target не подтверждает
совместимость окон, GPU, стандартной библиотеки, потоков или звука.
[Официальный Rust target](https://doc.rust-lang.org/rustc/platform-support/aarch64-nintendo-switch-freestanding.html)
имеет Tier 3. Его наличие само по себе не означает готовность `std` для зависимостей Horizon.

## Этапы и критерии приёмки

1. **Аппаратный тест libnx.** Собрать `.nro`; на Lite проверить
   отображение, оба стика, кнопки, доступность SD и выход через `+`.
   Текстовая консоль не проверяет GPU-рендерер.
2. **Rust runtime.** Выбрать и зафиксировать совместимый toolchain; выполнить
   на консоли Rust-код с выделением памяти, файлами, временем, потоками и синхронизацией.
   Отдельно проверить поддержку `std` и ABI взаимодействия с libnx.
   Текущий промежуточный результат: собран `no_std/core` ABI-тест с C/libnx;
   на Lite подтверждены 2000 проверок и 0 ошибок. Полноценная `std` официальным target не
   предоставляется; см. [ограничения и эксперимент](rust-runtime.md).
3. **Сетевой proof of concept.** Проверить libnx sockets/DNS, таймауты и источники
   случайных чисел. Выполнить Java server-list ping и получить MOTD/версию сервера
   до интеграции графики. После переноса runtime проверить login/configuration/play,
   компрессию и шифрование с использованием upstream протокола.
4. **GPU proof of concept.** Проверить применимость Switch Mesa/EGL и адаптации
   wgpu GLES либо отдельного backend на deko3d. Это варианты исследования,
   а не выбранные или доказанные решения. Показать текстурированный куб, depth buffer
   и обработку шейдеров, необходимых Horizon. Измерить CPU/GPU память и время кадра.
5. **Минимальная интеграция Horizon.** Зафиксировать полный upstream checkout,
   прочитать его AGENTS.md и инструкции модулей, отключить необязательные сервисы,
   определить границы замены winit/gilrs/cpal и системных вызовов.
   Сначала вывести один чанк, затем подключать мир и игровой цикл.
6. **Играбельность.** Игра с Java-клиентом на одном сервере: движение, чанки,
   сущности, инвентарь, установка/разрушение блоков, чат, reconnect. Локальный мир,
   сохранение, звук, suspend/resume. Установить бюджеты памяти и графики по измерениям на Lite.

Решение о графическом backend принимается после этапа 4. Сроки и FPS сейчас
оценивать рано: ни runtime, ни графический путь Horizon на консоли не проверены.
Первый тест намеренно использует C/libnx, чтобы отделить проверку оборудования от
неопределённости Rust runtime.

## Совместимость с Java-серверами

У Horizon уже есть [модуль Java-протокола](https://github.com/horizon-rust/horizon/blob/c09ecc9b0cb965f22a6fd389daaa5db76bdddb25/crates/horizon-protocol/src/lib.rs):
framing, компрессия, шифрование и состояния handshake/status/login/configuration/play.
[Манифест](https://github.com/horizon-rust/horizon/blob/c09ecc9b0cb965f22a6fd389daaa5db76bdddb25/crates/horizon-protocol/Cargo.toml)
указывает Java 26.3 / protocol 777. В
[списке серверов](https://github.com/horizon-rust/horizon/blob/c09ecc9b0cb965f22a6fd389daaa5db76bdddb25/crates/horizon-client/src/multiplayer.rs)
есть проверка совместимости протокола и опциональные адаптеры старых версий.
Это основание переиспользовать сетевой код, но не подтверждение работы порта на Switch.
В [net.rs](https://github.com/horizon-rust/horizon/blob/c09ecc9b0cb965f22a6fd389daaa5db76bdddb25/crates/horizon-client/src/net.rs#L830)
при отсутствии `auth` login использует `offline_uuid(name)`. Этот путь подтверждает
наличие offline-механики в сетевом слое; ограничения launcher/подготовки данных и UI
выбора профиля следует проверить отдельно при интеграции.

Минимальная матрица проверки:

- Контрольный vanilla Java-сервер версии, соответствующей выбранному upstream.
- Контрольный Paper-сервер совместимой версии.
- Offline-профиль: ник, сохранение выбора профиля на SD, отсутствие обязательного
  Microsoft-входа при запуске. Проверить правила имени и формирование offline UUID
  по правилам Java Edition, не подменяя серверный UUID.
- Вход без токена на контрольный `online-mode=false` сервер; при наличии серверного
  auth-плагина — ввод его команд через чат/экранную клавиатуру. Совместимость
  конкретных плагинов проверяется отдельно.
- Отдельный режим `online-mode=true`: Microsoft/Minecraft сессия, session join, шифрование;
  отдельно проверить HTTPS, сертификаты, корректные часы и источник энтропии на Switch.
- Соседний обычный Java-клиент видит игрока с Lite; изменения мира и инвентаря совпадают.
- Неверная версия, timeout, разрыв Wi-Fi и истёкшая сессия дают понятную ошибку;
  повторное подключение не требует перезапуска приложения.

Для сторонних публичных серверов и серверов с модами/античитом совместимость
проверяется отдельно. Наличие Java-протокола не обещает поддержку всех серверных модов.
Порт не должен требовать смены серверного ПО на Horizon. Offline-профиль работает
там, где сервер разрешает offline-вход, и не заменяет Minecraft-сессию на серверах
с `online-mode=true`. Выбор профиля должен явно различать offline и Microsoft;
способ Microsoft-входа на консоли ещё предстоит выбрать.
Offline-режим профиля не отменяет требования upstream к подготовке игровых данных
из пользовательской установки Minecraft.

## Источники начального теста

- [Официальный application Makefile](https://github.com/switchbrew/switch-examples/blob/master/templates/application/Makefile)
  — основа `platform/switch/probe/Makefile`, изменены имя, метаданные и предупреждения.
- [libnx console API](https://github.com/switchbrew/libnx/blob/master/nx/include/switch/runtime/devices/console.h).
- [libnx pad API](https://github.com/switchbrew/libnx/blob/master/nx/include/switch/runtime/pad.h)
  — default pad читает Handheld и первый подключённый контроллер.
- [Официальный Dockerfile devkitA64](https://github.com/devkitPro/docker/blob/master/devkita64/Dockerfile).

Файлы и ассеты Minecraft в этот репозиторий не включены. Способ подготовки данных
из пользовательской установки и сохранение авторизации Horizon нужно определить
на этапе интеграции.

## Performance, compatibility and implementation language

Runtime performance and Minecraft Java-server compatibility take precedence.
The intended client remains primarily Rust, reusing Horizon's architecture.
Application logic, protocol/world decoding, simulation, UI and renderer integration
should stay in Rust where practical. A small native C/C++ SDK bridge is acceptable
when it makes the Switch path faster or more compatible. The C hardware/network
probes are platform experiments, not a decision to rewrite the client in C.
Backend choices need Switch measurements; an installed library or host compile
alone does not establish performance or Java protocol compatibility.

## Structured world-data milestone

The separate 0.5.0 diagnostic adds no_std Rust NBT scanning and structural
registry/chunk/light decoding, keeping the existing C SDK bridge. Its host
connection to the user's Java 26.3 world checked 49 columns and 1176 sections
and resolved a raw block-state/biome sample below the acknowledged pose.
The user also confirmed WORLD_DATA_OBSERVED on Switch Lite. This does not
install terrain or justify PlayerLoaded.
Next work needs a retained world representation and a measured GPU path.
The [offline GPU inventory](switch-gpu-path-audit.md) confirms installed deko3d,
UAM and EGL/GLES libraries, but no native GPU rendering or acceleration is proven.

## GPU bring-up 0.6.0

The separate Rust GPU Probe builds and packages a Rust-generated animated triangle
through a deko3d 0.5.0 C SDK bridge, with original UAM shaders embedded in RomFS.
Rust owns geometry, time, palette and button behavior; C owns libnx/deko3d calls.
The bridge waits for queue idle before recycling shared command/uniform memory.
This serialized diagnostic does not measure renderer performance or select the
final backend. Native display, input response and teardown still need a device
result. World caching, meshing, assets and Horizon/wgpu integration remain open.
