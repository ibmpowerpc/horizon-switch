# CraftRustNX

Экспериментальный проект нативного порта [Horizon](https://github.com/horizon-rust/horizon)
на Nintendo Switch первого поколения. Основная тестовая консоль — **Switch Lite**,
среда запуска — **Atmosphère / hbmenu**, формат — `.nro`.

Обязательная цель — играть на серверах **Minecraft Java Edition** с поддерживаемым
Horizon протоколом. Приоритет — локальные **offline-профили** (ник без входа через
Microsoft) для серверов с `online-mode=false` и серверной авторизацией.
Для `online-mode=true` потребуется действительная Minecraft-сессия; Microsoft-вход
планируется отдельным режимом. Поддержку каждой версии сервера нужно проверять отдельно.

Реализованы аппаратный тест C/libnx, тест C/Rust ABI и compile-only аудит libc.
Все шесть проверок отдельного Rust std NRO прошли на Switch Lite.
Игровой клиент Horizon и GPU-рендерер ещё не перенесены. Тест использует текстовый framebuffer;
его успешный запуск не доказывает совместимость графического движка Horizon.

## Сборка аппаратного теста

С установленным и запущенным Docker:

```sh
bash scripts/build-switch.sh
```

С локальным devkitPro (`switch-dev`, настроены `DEVKITPRO` и `DEVKITA64`):

```sh
bash scripts/build-switch.sh native
```

Скрипт использует уже загруженный официальный образ `devkitpro/devkita64:20260219`.
Автоматические загрузки и сеть контейнера отключены для контроля расхода диска.
На другом компьютере сначала нужно проверить свободное место и отдельно загрузить
этот образ командой `docker pull devkitpro/devkita64:20260219`. Результат:
`dist/switch/craftrustnx/craftrustnx-probe.nro`.
Тег образа фиксирован; digest и версии пакетов проверенной сборки см. в
[журнале проверки](../../docs/verification.md).

Скопируй каталог `dist/switch/craftrustnx` в `sdmc:/switch/` и открой тест через hbmenu.
На экране должны обновляться счётчик кадров, маска кнопок и координаты обоих стиков.
`A` меняет цвет текста, `B` очищает маску уже проверенных кнопок, `+` возвращает в hbmenu.
Тест проверяет доступность корня SD, не записывая файлы.

Для проверки сообщи версию Atmosphère, версию системы, режим запуска hbmenu
(applet/application), работает ли ввод и корректен ли выход через `+`.

## Тест Rust + libnx

Аппаратный тест запускается на Switch Lite; пользователь подтвердил реакцию всех
кнопок. Следующий отдельный тест собирается командой:

```sh
bash scripts/build-switch.sh docker rust
```

Он использует уже установленный Homebrew Rust 1.100.0 и соответствующий rust-src,
собирает `core` в offline-режиме и связывает Rust staticlib с C/libnx.
`RUSTC_BOOTSTRAP=1` включён только внутри скрипта: это эксперимент с нестабильной
возможностью build-std, без смены глобального toolchain. Если локальных зависимостей
недостаточно, сборка остановится без загрузок. Скрипт сообщает фактический расход
места; прежние лимиты 10 GiB/512 МиБ сняты пользователем. Кэши сохраняются,
артефакты Rust исключены из git.

Результат: `dist/switch/craftrustnx/craftrustnx-rust-probe.nro`.
Его можно положить рядом с тестом кнопок. В hbmenu он называется
**CraftRustNX Rust Probe**. Ожидается `Rust ABI: PASS`, растущий `checks` и `errors: 0`
при нажатиях кнопок и движении стиков. `+` — выход.

Этот тест проверяет только вызовы C → Rust с аргументами и результатами `u64`.
На Switch Lite пользователь подтвердил **2000 checks, errors 0**.
Поддержка `std`, выделения памяти, файлов, потоков, сети и движка Horizon
им ещё не проверяется. Технические ограничения описаны в
[заметке о runtime](../../docs/rust-runtime.md).

## План порта

См. [технический план](../../docs/switch-port.md). Status-запрос уже подтверждён
на Switch Lite; далее нужны проверка входа с учётом версии сервера и отдельная
проверка GPU-вывода.

## Тест Rust std

```sh
bash scripts/build-switch.sh docker std
```

Результат: `dist/switch/craftrustnx/craftrustnx-std-probe.nro`. В hbmenu это
**CraftRustNX std Probe**. Up/Down выбирает проверку, A запускает одну проверку,
`+` выходит. Пункты проверяют выделение памяти, время, TLS, создание потоков,
синхронизацию и TLS-деструкторы рабочего потока. Пользователь подтвердил **PASS
для всех шести проверок на Switch Lite** 2026-11-15. Файлы, сеть и игровой движок этим NRO
не проверяются. Подробности — в [std-probe](std-probe/README.md).

## Сетевой status-тест

`bash scripts/build-switch.sh docker net` собирает отдельный
`craftrustnx-net-probe.nro`. X вводит адрес Java-сервера, Y — порт, A отправляет
status-запрос, B отменяет, Plus завершает после остановки потока. `DONE` означает
получение status-кадра; это ещё не вход в мир и не проверка offline-аккаунта.
Пользователь подтвердил `DONE` и узнаваемый ответ своего сервера на Switch Lite.
DNS может задержать отмену/выход.
Подробности и host-проверки — в [net-probe](net-probe/README.md).

## Диагностика offline login

`bash scripts/build-switch.sh docker login` собирает отдельный
`craftrustnx-login-probe.nro` для Java 26.3 / protocol 777. X — адрес,
Y — порт, L — offline-ник (по умолчанию `CraftRustNX`), A — подключиться.
`LOGIN_ACCEPTED` означает получение login-профиля: тест сразу закрывает
соединение, не переходя к конфигурации и игровому миру. Online-аутентификация
и шифрование в этом info-only варианте не реализованы. Он аппаратно подтвердил
запрос шифрования; дальнейший обмен выполняет вариант ниже. См.
[login-probe](login-probe/README.md).

## Зашифрованный offline login

`bash scripts/build-switch.sh docker encrypted-login` создаёт отдельный
`craftrustnx-encrypted-login-probe.nro`. В hbmenu это **CraftRustNX Encrypted
Login Probe 0.2.0**. Управление X/Y/L/A/B/Plus то же. Тест выполняет RSA/AES-обмен
без session join и показывает решение сервера. Сам флаг should_authenticate
не считается доказательством отказа offline-профилю. На Switch Lite подтверждены
`LOGIN_ACCEPTED`, `Encryption active: yes`, Error NONE и threshold 256
для тестового Java 26.3 LAN-мира. Это пока вход до configuration/play. Подробности —
[ENCRYPTED.md](login-probe/ENCRYPTED.md).

## Configuration и первый игровой пакет

`bash scripts/build-switch.sh docker configuration` собирает отдельную
**CraftRustNX Configuration Probe 0.3.0** — `craftrustnx-configuration-probe.nro`.
Она продолжает зашифрованный offline login, отвечает на служебные сообщения,
подтверждает configuration и останавливается на первом пакете play.
Ожидаемый результат: `PLAY_PACKET_SEEN`, `Error: NONE`, `Configuration ACK: yes`.
Registry/NBT пока принимаются как непрозрачные данные; мир не загружается.
Никакие resource packs не скачиваются, согласие с правилами автоматически не даётся.
Подробности и ограничения — в [CONFIGURATION.md](login-probe/CONFIGURATION.md).
На Switch Lite подтверждены `PLAY_PACKET_SEEN`, `Error: NONE`, ACK `yes`,
32 registry-пакета и первый play-пакет ID 50. Это ещё не игровой клиент.

Для следующих сборок сетевых проб общий начальный адрес — `192.168.0.12`
(`probe/include/probe_defaults.h`); X по-прежнему позволяет его изменить.
Новые сетевые пробы должны использовать этот общий default. Порт выбирается
отдельно: LAN-мир может назначить новый порт при повторном открытии.

## Ограниченная play-сессия

`bash scripts/build-switch.sh docker play-session` создаёт отдельную
**CraftRustNX Play Session Probe 0.4.0** — `craftrustnx-play-session-probe.nro`.
Начальный IP `192.168.0.12`, X меняет адрес, Y — текущий LAN-порт.
Проба разбирает Login и позицию, подтверждает телепортацию, принимает чанки как
непрозрачные данные, подтверждает batch и отвечает на keep-alive.
Ожидаемый итог — `PLAY_SESSION_OBSERVED`, `Error: NONE`, минимум по одному
Pose ACK, keepalive, opaque chunk и batch ACK. Возможна пауза около 15 секунд
до keep-alive; весь тест ограничен 30 секундами после DNS.
`PlayerLoaded` и движение не отправляются, мир не загружается и не отображается.
На Switch Lite этот этап прошёл: `PLAY_SESSION_OBSERVED`, Error `NONE`,
1 Pose ACK, 49 opaque chunks, 41 batch ACK и 1 keepalive. Подробности —
[PLAY-SESSION.md](login-probe/PLAY-SESSION.md).

## Структурный разбор данных мира

`bash scripts/build-switch.sh docker world-data` собирает отдельный
**CraftRustNX World Data Probe 0.5.0**. Rust разбирает NBT, выбранные registry,
секции чанков, палитры и слои света. Ожидается `WORLD_DATA_OBSERVED`,
`Error: NONE`, checked chunks/sections больше нуля и образец state/biome ID
под игроком. Адрес по умолчанию — `192.168.0.12`; порт вводится через Y.
Проверка с настоящим Java 26.3 сервером с компьютера прошла; пользователь также
подтвердил `WORLD_DATA_OBSERVED` на Switch Lite. Это ещё не загрузка мира или рендер; PlayerLoaded не отправляется.
Подробности — [WORLD-DATA.md](login-probe/WORLD-DATA.md).

## Первый GPU-тест

`bash scripts/build-switch.sh docker gpu` создаёт отдельный
`craftrustnx-gpu-probe.nro` и `craftrustnx-gpu-notices.txt`. В hbmenu выбери
**CraftRustNX Rust GPU Probe 0.6.0**, затем нажми A на экране инструкций.
Ожидается движущийся цветной треугольник на тёмном фоне. В графическом режиме:
A — пауза, B — палитра, X — сброс, Plus — результаты; ещё раз Plus — hbmenu.
Геометрию и управление задаёт Rust, GPU вызывается через небольшой SDK-мост.
Это отдельный smoke test с ожиданием GPU перед перезаписью буферов, не FPS benchmark.
Пользователь подтвердил на Switch Lite видимую анимацию, работу A/B/X и выхода
через Plus. Измерений FPS/памяти пока нет.
См. [gpu-probe](gpu-probe/README.md).

## Настоящая геометрия моделей: Rust Model Mesh Probe 0.8.0

`bash scripts/build-switch.sh docker terrain` собирает отдельную пробу девяти
статичных моделей из пользовательских ресурсов Java 26.3. Геометрию запекает
Rust baker Horizon, Rust на Switch проверяет mesh и управляет камерой; native
SDK-мост загружает вершины и использует depth buffer. Текстуры, alpha, освещение
мира и live chunks пока не реализованы. Подготовка private mesh и управление:
[terrain-probe](terrain-probe/README.md). NRO, mesh и notices переданы на SD Card
с обратным чтением. Пользователь подтвердил девять фигур и работу всех
указанных controls на Switch Lite; это пока статичная сцена без текстур.

## Снимок полной колонки

`bash scripts/build-switch.sh docker column-snapshot` собирает отдельный
**CraftRustNX Column Snapshot Probe 0.7.0**. Он сохраняет полную колонку raw
block/biome ID через Rust в статических буферах и сверяет образец под игроком.
Ожидается `COLUMN_SNAPSHOT_OBSERVED`, `Error: NONE`, snapshot status0 и
`Snapshot sample agrees: yes`. Это снимок принятого пакета, без применения
последующих block/light updates, рендера мира или PlayerLoaded. Host fixtures
и live-проверка Java 26.3 с компьютера прошли: 24 секции, 98,304 состояния блоков
и 1,536 biome cells; образец совпал. Фото Switch Lite также подтвердило `COLUMN_SNAPSHOT_OBSERVED`, Error NONE,
24 сохранённые секции и совпадение sample. Подробности —
[COLUMN-SNAPSHOT.md](login-probe/COLUMN-SNAPSHOT.md).
