# Проверка начального теста

Сборка и аппаратная проверка учитываются раздельно.

## Компьютерная сборка (devkitA64)

**Команда:** `bash scripts/build-switch.sh`

**Контейнер:** `devkitpro/devkita64:20260219`
**RepoDigest:** `sha256:1fc388c3a0d34bd2045a6dadcb1020e069d5f876a187fd705de14b4440c00282`
**ОС контейнера:** Debian GNU/Linux 12 (bookworm)

**Установленные пакеты (из образа):**
- devkitA64: `r29.2-1` (GCC 15.2.0)
- libnx: `4.12.0-1`
- switch-tools: `1.13.1-1`

**Данные о диске (до и после):**
- Host свободно: ~18 GiB; проект с результатами сборки занимает 4.1 MiB.
- Docker image: `devkitpro/devkita64:20260219` — 2.722 GB (скачан один раз)

## Результат сборки

**Артефакт:** `dist/switch/craftrustnx/craftrustnx-probe.nro`
**Размер:** 215237 байт
**SHA256:** `f93f7f0d4f893cea3eea498b5239a1e3ea0710d48b2026344a1a6277a93e73e9`

**Валидация NRO:**

- NRO magic: `4e 52 4f 30` (`NRO0`) на смещении 0x10 — OK

**Исходный ELF:** `platform/switch/probe/craftrustnx-probe.elf`

- ELF header: `7f 45 4c 46`, ELF64 little endian
- Machine: AArch64
- Type: DYN (Position-Independent Executable)
- 36 section headers, 4 program headers
- .text: 0x00021348 bytes (~136 KB)

## Аппаратная проверка

2026-11-15: NRO передан через DBI/MTP на SD-карту Switch в
`sdmc:/switch/craftrustnx/craftrustnx-probe.nro`. Файл скачан обратно через MTP:
215237 байт, побайтовое совпадение и SHA256 подтверждены. Запуск приложения
на устройстве пока не подтверждён пользователем.

**Статус:** КОМПИЛЯЦИЯ УСПЕШНА; ЗАПУСК И КНОПКИ ПОДТВЕРЖДЕНЫ ПОЛЬЗОВАТЕЛЕМ

- 2026-11-15: пользователь запустил NRO на Switch Lite и сообщил:
  «программа реагирует на все кнопки».
- Работоспособность запуска и кнопочного ввода подтверждена на реальной консоли.
- Координаты обоих стиков, выход через `+`, доступность SD и режим запуска hbmenu
  отдельно пока не подтверждены. Версии системы и Atmosphère не сообщены.

## Блокеры для следующего этапа

- Дальнейшие аппаратные проверки выполняются пользователем на его Switch Lite.
- Horizon runtime и GPU-рендерер пока не перенесены (см. README.md)

## Следующий шаг

Проверить путь сборки Rust runtime под нативный NRO с учётом лимита диска,
затем подготовить отдельный минимальный тест сети для Java-серверов.

## Rust + libnx ABI probe (2026-11-15)

- Команда: `bash scripts/build-switch.sh docker rust`.
- Host: Homebrew Rust 1.100.0 с соответствующим rust-src; экспериментальный
  `RUSTC_BOOTSTRAP=1`, `build-std=core`, offline/locked, один job.
- CPU Cortex-A57, PIC, release, `panic=abort`, без debug/LTO.
- Связан с C/libnx через уже загруженный devkitA64 Docker image; сеть контейнера
  и автоматический pull отключены. Внешние зависимости не загружались.
- Результат: `dist/switch/craftrustnx/craftrustnx-rust-probe.nro`, 215237 байт.
- SHA256: `29b7bb94cbbaf51cc20861facaffdda60927454ad8d5940cf650c78e44a1a435`.
- Проверены `NRO0` на 0x10, ELF64 AArch64 DYN и наличие символа
  `craftrustnx_rust_mix` в конечном ELF.
- Исходный тест кнопок также пересобран отдельно; его новая SHA256:
  `9bc002aa9e160f4d76557479c853d388de4fe553249a698137225ff8d87fc590`.
  На Switch ранее был передан и проверен предыдущий бинарный файл с SHA256,
  указанной в начальном отчёте выше; этот файл на SD не заменялся.
- Артефакты Cargo: 267 МиБ, в том числе кеш первоначальной сборки без новых флагов;
  оба NRO в dist вместе занимают 424 КиБ. В конце проверки на host свободно 17 GiB.
- Новый Rust-тест передан через DBI/MTP в
  `sdmc:/switch/craftrustnx/craftrustnx-rust-probe.nro` 2026-11-15.
  Файл скачан обратно: размер, побайтовое совпадение и SHA256 подтверждены.
  Предыдущий тест кнопок на SD не заменялся.
- **Rust ABI-тест запущен на Switch Lite**: 2026-11-15 пользователь сообщил
  «2000 checks, errors 0». Таким образом, аппаратно подтверждены 2000 сравнений
  C → Rust без обнаруженных несовпадений. Выход, полный диапазон стиков и
  длительная стабильность отдельно не подтверждались.

Критерий аппаратной проверки: `Rust ABI: PASS`, счётчик checks растёт,
errors остаётся 0 при движении стиков/нажатиях кнопок, `+` возвращает в hbmenu.
Успех подтверждает только примитивный C → Rust ABI; `std`, heap, файлы,
потоки, сеть и игровые зависимости Horizon не проверены.

## Интеграция в GitHub-форк

- Репозиторий: https://github.com/ibmpowerpc/horizon-switch ; Switch-порт развивается в main.
- Основа: upstream `41cdc1a8d2fd6dbcc8b56a831cf71f284f7973d3`; дерево движка сохранено.
- Прежний приватный `ibmpowerpc/craftrustnx` сохранён; его remote — `bootstrap-origin`.
- Для экономии места используется разреженный checkout и shallow/filtered fetch.
- Standalone manifests тестов получили собственный `[workspace]`, чтобы не
  загружать весь Cargo workspace Horizon. Проверены metadata patched workspace,
  повторная сборка обеих NRO и строгий patched ABI-аудит: 69/69.
- Проверены C-знаковость typedefs и порядок полей в дампе pthread-инициализаторов.
- Артефакты Rust занимают около 406 МиБ; свободно около 17 GiB.
- NRO после переноса имеют те же SHA256, что в предыдущей секции; повторный
  аппаратный запуск после переноса Git-истории не проводился.
- Полный Horizon не собирался и его игровые тесты не запускались; код движка
  этим коммитом не меняется. Ассеты Minecraft и Legacy4J не добавлены.

## Первая сборка std и кодек status ping

- Rust 1.100.0 std собрана offline с проектным target и исправленной libc.
  Проверка unit graph подтверждает одну libc из локального vendor source,
  включая зависимость самой std. Системные исходники и Cargo registry не менялись.
- Удалены три наших устаревших кеша core (примерно 384 МиБ); NRO и результаты
  ABI-аудитов сохранены. Список удалённых generated-каталогов записан локально
  в std-probe/build/cache-cleanup.json. После очистки сборка std заняла 213 МиБ.
- Добавлен std-probe NRO с шестью выбираемыми проверками: ALLOC, TIME, TLS main,
  THREAD, SYNC, TLS destructor. По умолчанию ничего не выполняется; сначала ALLOC.
- При линковке обнаружен отсутствующий sysconf. Адаптер поддерживает только
  _SC_PAGESIZE=8, возвращает 4096, а остальные запросы — -1/EINVAL. Номер проверен
  в C и Rust. Основная pthread-реализация взята из libsysbase, без подмен.
- Артефакт: dist/switch/craftrustnx/craftrustnx-std-probe.nro, 506053 байт.
  SHA256: `7f9c0400bce1fed8fc7f54cbe976f799171e27117568ffed7b6b73219283f978`. Проверены NRO0 и ELF64 AArch64 DYN.
- Host-тест логики std-probe прошёл, clippy без warnings. Сам по себе он не проверяет
  runtime на Switch; последующий аппаратный результат записан ниже. Работа с файлами/сетью ещё не проверялась.
- Кодек status ping проверен по независимым wire vectors: handshake + status-request
  имеют отдельные length prefixes, server status response id=0 (id=1 — pong).
  Семь групп тестов проходят, включая 65536 коротких входов; clippy без warnings.
  Кодек не открывает сокет и не проверяет login/gameplay.
- После очистки также проверена повторная сборка прежнего no_std Rust-теста:
  core восстановился offline, NRO собрался. Восстановлен один нужный кеш core
  (shared target теперь около 151 МиБ вместо 23); общий эксперимент остаётся
  ниже 512 МиБ. Удаление кешей не лишило предыдущий тест возможности пересборки.
- std-probe передан через DBI/MTP в `sdmc:/switch/craftrustnx/craftrustnx-std-probe.nro`.
  Выполнено обратное чтение: побайтовое совпадение, 506053 байт, SHA256 совпадает
  с записью выше. Предыдущие тесты на SD не заменялись. Аппаратный результат ALLOC
  подтверждён пользователем, как и остальные пять проверок ниже.

## Аппаратный результат std-probe на Switch Lite

Пользователь последовательно сообщил PASS для всех шести выбранных проверок:
ALLOC (Vec/String), TIME, TLS main thread, THREAD spawn/join,
SYNC mutex/condition и TLS worker destructor. Таким образом, базовые функции
этой конфигурации std/libc/libnx подтверждены на реальной Switch Lite.
Это не полная проверка std: файлы, сеть, случайность, длительная стабильность
и игровой движок Horizon ещё не тестировались.

## Сетевой status-probe: сборка и host-проверка

2026-11-15 собран `craftrustnx-net-probe.nro` (293061 байт), SHA256:
`8be6fecdb471a6b7dba4f5545ef59715d3815565e8b536267cbf23642e8f64dc`.
Проверены `NRO0` и ELF64 little-endian AArch64. Сборка использовала существующий
образ devkitA64 без сети/загрузок и повторно использовала core-кеш no_std probe.

- Rust FFI над существующим status-кодеком: 7 host-тестов, clippy `-D warnings`
  и fmt прошли. Panic вызывает C abort; host-only personality shim не включается
  в Switch-архив с abort-only core.
- Реальный C TCP transport связан с host Rust staticlib. Все 8 loopback-фикстур
  прошли: корректный ответ, отправка ответа частями, EOF неполного кадра,
  неверный packet ID, превышение длины, отмена до DNS, отмена при чтении,
  истечение 10-секундного deadline. В тесте отмена заняла около 0.21 с,
  deadline — около 10.02 с; это измерения на host, не на Switch.
- Исправлены чтение устаревшего terminal state после повторного запуска,
  обработка threadStart failure, отмена/ожидание потока при выходе applet,
  EINTR, подавление SIGPIPE на поддерживающих платформах и сохранение errno
  до освобождения addrinfo.
- Артефакты экспериментов занимают около 422 МиБ из 512 МиБ; свободно около
  17 GiB при резерве 10 GiB. Предыдущие NRO сохранены.

На этапе сборки аппаратный запуск сетевого NRO ещё ожидался; последующий результат
пользователя записан ниже.
Это native C/libnx sockets + Rust framing, не std::net. Поддерживаются IPv4
и только первый адрес DNS; SRV и IDNA отсутствуют. DNS может блокировать:
10 секунд отсчитываются после разрешения имени, отмена/выход ждут getaddrinfo.
Текст JSON показывается с ограничением и санитизацией, синтаксис JSON не разбирается.
Вход, offline login и игровой мир этим этапом не реализованы.

## Передача сетевого NRO через MTP

2026-11-15: DBI обнаружен как Nintendo Switch / Lite, VID 057e, PID 201d.
Новый `craftrustnx-net-probe.nro` передан только в `1: SD Card` →
`switch/craftrustnx/`, object ID 16777262. Предыдущие NRO не заменялись.
Обратное чтение подтвердило побайтовое совпадение: 293061 байт, SHA256
`8be6fecdb471a6b7dba4f5545ef59715d3815565e8b536267cbf23642e8f64dc`.
Временный helper и readback удалены. Результат аппаратного запуска записан ниже.

## Аппаратный результат status-probe на Switch Lite

2026-11-15 пользователь сообщил `stage done`: запрос вернул текст, содержащий
его Minecraft-ник, и пользователь счёл ответ правильным. Это подтверждает
успешный TCP-обмен и разбор status-кадра C/libnx + Rust-кодеком на консоли.
Сам JSON/его поля не предоставлены: точная версия и протокол сервера пока
не установлены. Ранее пользователь указал порт открытого мира 52246;
точный использованный маршрут (локальная сеть/Tailscale-посредник) не уточнён.
Ник, адрес сервера и текст ответа в репозиторий не записываются.

Этот результат не проверяет DNS отдельно (мог использоваться IP), корректность
синтаксиса JSON, login, offline-профили, конфигурацию/play, шифрование/компрессию,
std::net или длительную стабильность. Следующий сетевой этап требует версии
сервера; status-совместимость не означает совместимость входа по протоколу 777.

## Версия тестового мира и сборка login-probe

Пользователь уточнил версию тестового мира: **Java 26.3**. Протокол 777 выбран
по pinned исходникам Horizon; успешный status-запрос сам по себе не подтверждает
совместимость login. Адрес/ник из фотографии не публикуются.

2026-11-15 собран `craftrustnx-login-probe.nro`, 428229 байт, SHA256:
`412989f3c46e5bd2f29384efb1436493fcdad97797b39154526717bee9416605`.
Проверены NRO0, ELF64 LE AArch64 DYN, отсутствие DT_TEXTREL и размещение всех
433 динамических RELA/RELR-релокаций только в writable LOAD-сегменте.

- Rust no_std login staticlib собрана offline установленным Rust 1.100.0,
  с повторным использованием core-кеша. Локально доступные miniz_oxide 0.8.9
  и adler2 2.0.1 используются без alloc; ничего не загружалось.
- Пройдены 31 Rust-тест, clippy `-D warnings` и fmt. Векторы проверяют offline
  UUID v3, Hello/LoginFinished с session UUID, property caps, framing и zlib.
  C ABI обнуляет event, затем пишет поля отдельно, не копируя padding структуры.
  Возвращаемое имя ограничено 1..16 ASCII letters/digits/underscore.
- Пройдены 18 loopback TCP-фикстур настоящего C transport + Rust archive:
  raw login profile, coalesced/fragmented сжатый ответ, raw compression envelope,
  отключение сжатия, disconnect, запрос шифрования, query/cookie, неизвестный ID,
  checksum, превышение распакованного/внешнего размера, лимит 16 событий, EOF,
  отмена до DNS/при чтении и deadline. Fixtures проверяют закрытие без LoginAcknowledged.
  Отмена при чтении около 0.21 с, deadline около 10.01 с на host.
- Оба packet-буфера размещены в статическом C job. Удалён 64 KiB automatic
  recv buffer, поток получает 128 KiB stack для Rust/miniz. C-сборка использует
  `-Wframe-larger-than=32768`; это не измерение полного Rust call stack.
- Первый линк ошибочно использовал мои compile-only C-объекты без fPIE из
  рабочего каталога Makefile. После удаления только этих временных объектов
  штатная сборка с fPIE прошла. Диагностический ELF с ослабленным linker flag
  удалён; поставляемый NRO собран без ослабления read-only relocation checks.
- Общий budget helper учитывает target/build, готовые ELF/NRO/NACP/maps/listings
  и dist без повторного счёта вложенных каталогов: около 457 МиБ из 512 МиБ,
  свободно около 17 GiB (резерв 10 GiB). Старые NRO сохранены.

Аппаратный login ещё не проверен. `LOGIN_ACCEPTED` означает только получение
структурно проверенного LoginFinished; probe закрывается до configuration/play,
не отправляя acknowledgement. Microsoft-аутентификация/шифрование, ответы на
query/cookie, конфигурация и игровой мир не реализованы. Любой EncryptionRequest
завершает тест как AUTH_OR_ENCRYPTION_REQUIRED. IPv4/первый адрес, 64 KiB body cap;
DNS может блокировать отмену, 10-секундный deadline начинается после DNS.

## Передача login-probe через MTP

2026-11-15 новый `craftrustnx-login-probe.nro` передан через DBI/MTP
в `1: SD Card` → `switch/craftrustnx/`, object ID 16777263.
Обратное чтение подтвердило побайтовое совпадение: 428229 байт, SHA256
`412989f3c46e5bd2f29384efb1436493fcdad97797b39154526717bee9416605`.
Предыдущие NRO не заменялись. Временный helper/readback удалены.
Аппаратный результат login ещё ожидается.

## Аппаратный login: запрос шифрования и диагностическая версия 0.1.1

2026-11-15 пользователь сообщил Stage `AUTH_OR_ENCRYPTION_REQUIRED`, Error `NONE`.
Подтверждён приём и разбор EncryptionRequest на Switch Lite, но не LoginFinished.
Старый NRO не показывал should_authenticate: нельзя сделать вывод о требовании
сессии или настройке online-mode. Постоянная верхняя надпись про LOGIN_ACCEPTED
не является результатом: результат показывает только строка Stage.

В версии 0.1.1 флаг передаётся через C ABI и отображается отдельно как
`Server should_authenticate: true/false`. Поле использует offset 69, размер
lp_event остаётся 72 байта. Верхняя надпись заменена пояснением «The Stage line
shows the result». NACP title — `CraftRustNX Login Info Probe`; изменение
Makefile теперь обновляет metadata через явную зависимость NACP.

Пройдены 31 Rust-тест, clippy/fmt и 19 TCP-фикстур, включая оба значения флага.
Новый NRO: 428229 байт, SHA256
`0a23c954a9e77860e1e9a5fdf83ee39d1c480990bea6cb4e60c45e348d6699e1`.
Проверены NRO0 и NACP title/version. Предыдущий артефакт сохранён локально как
`craftrustnx-login-probe-v0.1.0.nro`; на SD он пока остаётся под прежним именем.
На этапе сборки новая версия ещё не была передана/запущена. Шифрование и online-аутентификация
этим изменением не реализованы. Артефакты около 457 МиБ, свободно около 16.5 GiB.

## Передача Login Info Probe 0.1.1

2026-11-15 файл передан через DBI/MTP в `1: SD Card` →
`switch/craftrustnx/craftrustnx-login-info-probe.nro`, object ID 16777264.
Прежний login-probe и остальные тесты не заменялись. Обратное чтение:
побайтовое совпадение, 428229 байт, SHA256
`0a23c954a9e77860e1e9a5fdf83ee39d1c480990bea6cb4e60c45e348d6699e1`.
Временные helper/readback удалены. Полученное затем значение флага записано ниже.

## Аппаратный результат Login Info Probe 0.1.1

На фотографии пользователя от 2026-11-15 видны:
Stage `AUTH_OR_ENCRYPTION_REQUIRED`, `Server should_authenticate: true`,
Error `NONE`, Compression `-1`, Frames `1`, Received `174 bytes`.
Это подтверждает, что в этом подключении сервер запрашивает session authentication
и шифрование. Адрес, порт и профиль из фото не публикуются.

Тест правильно остановился после EncryptionRequest; LoginFinished/LOGIN_ACCEPTED
не получены. Общая настройка сервера из этого флага не устанавливается.
Дальнейшее поведение сервера этим флагом не установлено: probe остановился до
ответа на запрос. Первоначальный вывод об обязательной аккаунтной сессии был
слишком категоричным. Пользователь уточнил: тот же мир, открытый через
«Открыть для сети», уже принимал offline-профили через TLauncher. Следующий
диагностический этап выполнит криптографический обмен без session join и запишет
реальное принятие либо отказ сервера. LOGIN_ACCEPTED пока не подтверждён.

## Зашифрованный offline login probe 0.2.0

2026-11-15 собран отдельный `craftrustnx-encrypted-login-probe.nro`, 563397 байт,
SHA256 `67a9b8c9c1fd3c1ff274cc1f61f1d05c931dd2fb0aa38f5f2dab81e6377c9405`.
NACP title/version: CraftRustNX Encrypted Login Probe / 0.2.0. Проверены NRO0,
ELF64 LE AArch64 DYN, отсутствие DT_TEXTREL; все 571 динамических RELA/RELR
релокаций находятся в writable LOAD-сегментах.

- SDK: установленная mbedTLS 2.28.10 шифрует RSA PKCS#1 v1.5; libnx CSRNG
  генерирует секрет/паддинг с проверкой Result; libnx AES-block используется
  общей CFB8-обёрткой. DER ограничен 1024 байт, RSA 1024/2048, challenge 1..k-11.
  Системные исходники не менялись, загрузок/установок нет.
- Проверены null outputs до разыменования, cleanup partial key parse, проверки
  RSA public key/DER length, очистка секрета при любом отказе и balanced CSRNG.
  Host OpenSSL backend использует EVP; CFB8-алгоритм общий с SDK backend.
- 34 Rust-теста, clippy/fmt; 19 обычных TCP-регрессий; 6 CFB8-сценариев с NIST
  vector; 13 зашифрованных TCP-фикстур. Все 13 дополнительно прошли ASan/UBSan
  для C-кода. Это host-тесты, не выполнение SDK RNG/RSA/AES на Switch.
- TCP-сервер independently расшифровывает RSA secret/challenge, сверяет challenge,
  затем шифрует поток OpenSSL CFB8. Проверены flag false/true + принятие профиля,
  RSA2048, fragmented/coalesced compressed frames, отказ сервера, checksum,
  повторный Hello, partial EOF, отмена после Key, malformed/trailing DER,
  RSA512 и слишком длинный challenge. Синтетические ключи создаются только в
  ignored временном каталоге и удаляются после теста.
- EncryptionResponse отправляется plaintext; после завершения send включается
  RX AES. Остаток буфера расшифровывается один раз, каждый следующий recv — только
  по новым байтам. Лимит 16 событий, body 64 KiB и исходный deadline сохраняются.
- Флаг should_authenticate записывается отдельно от результата. В этом offline
  диагностическом режиме session join не отправляется: принятие/отказ определяет
  сервер. LOGIN_ACCEPTED по-прежнему завершает тест до configuration/play.
- Все артефакты около 466 МиБ из 512; свободно около 16 GiB, резерв 10 GiB.
  Ранее переданные NRO сохранены отдельно. SDK-криптография и реальный
  зашифрованный LoginFinished на Switch пока не проверены.

## Передача Encrypted Login Probe 0.2.0 через MTP

2026-11-16 новый `craftrustnx-encrypted-login-probe.nro` передан через DBI/MTP
в `1: SD Card` → `switch/craftrustnx/`, object ID 16777265. Прежние NRO не заменялись.
Обратное чтение подтвердило побайтовое совпадение: 563397 байт, SHA256
`67a9b8c9c1fd3c1ff274cc1f61f1d05c931dd2fb0aa38f5f2dab81e6377c9405`.
Рядом добавлены `craftrustnx-encrypted-notices.txt` и `craftrustnx-Apache-2.0.txt`
(object IDs 16777266/16777267), оба также проверены обратным чтением.
Временный helper/readbacks удалены. Аппаратный зашифрованный login ещё ожидается.

## Аппаратный результат Encrypted Login Probe 0.2.0

2026-11-16 фотография пользователя подтвердила на Switch Lite:
Stage `LOGIN_ACCEPTED`, `Server should_authenticate: true`, Encryption active `yes`,
Error `NONE`, Compression threshold `256`, Frames `3`, Received `226 bytes`.
Сервер вернул тестовый профиль игрока `CraftRustNX`. Локальный адрес, порт
и session UUID из фотографии не публикуются.

Это аппаратное подтверждение базового пути SDK CSRNG → RSA PKCS#1 v1.5 →
AES-CFB8 receive → compression framing → полный LoginFinished с 16-byte session UUID.
Настоящий Java 26.3 LAN-мир принял offline-профиль без session join, несмотря на
флаг true. При этом не проверены AES-CFB8 send, большие/реально zlib-сжатые тела
на устройстве, другие серверы/версии, configuration/play и длительная стабильность.

Подписи `Server name` / `Server UUID` в NRO 0.2.0 были ошибочны: UI выводит
job.event.name/uuid из GameProfile, т.е. имя и UUID принятого **игрока**.
В исходниках они заменены на `Accepted player` / `Player UUID`. Новый NRO ради
этой косметической правки пока не собирался и не передавался; сохранены артефакт
0.2.0 и его предыдущая SHA256-запись. Configuration acknowledgement не отправлялся,
тест штатно закрыл соединение до конфигурации и входа в игровой мир.

## Configuration → первый play-пакет: подготовка 0.3.0

2026-11-16 собран отдельный `craftrustnx-configuration-probe.nro`, 571589 байт,
SHA256 `04ee8e34468617a504e2da4b84ebce31ba86e8bc2764e93335eef50a57fe94f5`.
NACP: CraftRustNX Configuration Probe / 0.3.0. Проверка
`tools/verify_switch_artifact.py` подтвердила NRO0, ELF64 LE AArch64 DYN,
отсутствие DT_TEXTREL и 580 runtime RELA/RELR-релокаций в writable LOAD.
Проверка также отклоняет несовпадающий title. Прежний аппаратно проверенный
зашифрованный login NRO сохранён с исходной SHA256.

- Configuration-кодек использует общий bounded frame decoder с отдельными
  лимитами: 2^21−1 framed / 8 MiB decoded. 50 Rust-тестов прошли, включая
  прежние 34; clippy/fmt чистые. При родительском review исправлена лишняя длина
  в клиентском кадре без сжатия; независимая raw TCP-проверка подтвердила wire.
- Прежние 19 TCP-регрессий и 13 зашифрованных login-сценариев прошли с обновлённым
  транспортом. Четыре дополнительные loopback-проверки без EncryptionRequest
  подтвердили ACK, brand, ClientInformation, empty KnownPacks и play-переход
  с compression −1/0/1/256; C-код исполнялся с ASan/UBSan без ошибок.
- Зашифрованный TX применяется ко всему кадру один раз, до цикла partial send;
  receive-tail сохраняется после login при переходе на heap-буферы ~10 MiB.
  Cipher/compression не сбрасываются между фазами. Отмена/ошибки закрывают
  сокет и освобождают heap/AES до terminal release.
- Known packs всегда пустые; resource packs отклоняются; CodeOfConduct не
  принимается автоматически; Transfer не открывает новое соединение. Registry,
  tags, NBT и первый play payload остаются непрозрачными данными.
- Успехом этого диагностического этапа будет только `PLAY_PACKET_SEEN` при
  `Error: NONE` и `Configuration ACK: yes`. Это не подтверждение загруженного
  мира или готового игрового клиента. Аппаратная проверка ещё не выполнена.

- 12 независимых зашифрованных configuration-фикстур прошли обычный запуск
  и повтор с ASan/UBSan для C-кода (Rust/OpenSSL не instrumented; LeakSanitizer
  на этом macOS не поддерживается, detect_leaks=0). Проверены
  полные brand/ClientInformation, cookie identifier + absent payload, непустой
  known-pack offer → пустой ответ, keepalive/pong, resource-pack decline,
  >64 KiB registry, оба фазовых перехода с coalesced/fragmented RX, threshold 0,
  отсутствие compression, malformed control, unknown ID, CodeOfConduct,
  Transfer, отказ сервера после обязательного pack и детерминированная отмена.
  При review фикстуры исправлены обязательный cookie key, pack bool fields,
  Disconnect ID/NBT и неверная попытка SetCompression в configuration.
  Производственный клиент корректно отклонял исходный неполный cookie-запрос.

Передача нового NRO через MTP ещё не выполнена. Артефакты около 475 МиБ,
свободно 15.8 GiB; дополнительных загрузок нет.

## Передача Configuration Probe 0.3.0 через MTP

2026-11-16 `craftrustnx-configuration-probe.nro` передан через DBI/MTP
в `1: SD Card` → `switch/craftrustnx/`, object ID 16777268.
Обратное чтение подтвердило побайтовое совпадение: 571589 байт, SHA256
`04ee8e34468617a504e2da4b84ebce31ba86e8bc2764e93335eef50a57fe94f5`.
Старые NRO не заменялись. Временный helper и readback удалены после проверки.
Аппаратный запуск configuration ещё ожидается: успешная передача не означает
прохождения этого этапа на Switch.

## Аппаратный результат Configuration Probe 0.3.0

2026-11-16 фотография пользователя подтвердила на Switch Lite:
Stage `PLAY_PACKET_SEEN`, Error `NONE`, Configuration ACK `yes`, Encryption
active `yes`, Server should_authenticate `true`, Compression `256`.
Frames `41`, Received `39559 bytes`, configuration/play packets `38`, registry
packets `32`, decoded configuration/play bytes `216772`. Первый play-пакет:
ID `50`, body `110 bytes`. Принят тестовый профиль CraftRustNX.
Session UUID из фотографии не публикуется.

Пользователь установил причину предыдущего таймаута с нулём принятых байтов:
на серверном хосте не был подключён Wi-Fi. После восстановления сети текущий
NRO без изменений прошёл этап. Это подтверждает на устройстве зашифрованные
ответы клиента, получение и распаковку configuration-данных, Finish ACK и
переход к первому полному play-кадру. Registry/NBT и play body остаются
непрозрачными; корректность их схем, загрузка мира и игровой процесс не проверены.

По просьбе пользователя для следующих сборок сетевых проб начальный адрес
`192.168.0.12` вынесен в общий `probe/include/probe_defaults.h`; адрес можно
изменить кнопкой X. Порт автоматически не фиксируется на порту из фотографии,
поскольку LAN-порт может меняться. Аппаратно проверенный NRO 0.3.0 и его SHA256
сохранены: сборка ради изменения начального адреса не выполнялась.

Оба UI-источника с новым общим default прошли SDK `-fsyntax-only` с
`-Wall -Wextra -Werror` в существующем offline Docker image. Новых build-артефактов
не создавалось; на хосте остаётся около 15.8 GiB свободного места.

## Play Session Probe 0.4.0: host-проверки и сборка

2026-11-16 собран отдельный `craftrustnx-play-session-probe.nro`, 575685 байт,
SHA256 `c429b553268dff4395d76e1812b23ab57be6f6ff85021c6ae9e3e8428dd80dc4`.
NACP title/version: CraftRustNX Play Session Probe / 0.4.0. Проверены NRO0,
ELF64 LE AArch64 DYN, отсутствие DT_TEXTREL и все 588 runtime RELA/RELR-релокаций
в writable LOAD. Новый NRO использует общий editable IP default `192.168.0.12`.
Ранее аппаратно проверенные NRO не заменялись.

- 64 Rust-теста прошли (прежние 50 + 14 play), clippy/fmt чистые. Play ABI:
  state 320 bytes / align 8, event 32 bytes. State/reply не меняются при ошибке;
  event обнуляется, поля сохраняются без копирования неинициализированного padding.
- Независимая TCP-фикстура проверила полные wire-ответы и поймала отсутствующий
  pitch clamp в начальном кодеке. Исправлен clamp [-90,90] по исходнику
  `position_packet_pitch`. Также устранено переполнение при предыдущем игровом
  режиме i32::MIN: optional wire-code проверяется в диапазоне 0..4 до вычитания.
- 21 новый loopback-сценарий прошёл обычный запуск и ASan/UBSan для C-кода;
  Rust/OpenSSL не instrumented, LeakSanitizer на этом macOS не поддерживается.
  Синтетический сервер независимо шифрует/расшифровывает непрерывный CFB8-поток,
  проверяет Login, полный pose ACK, relative correction/clamp, routing play IDs,
  >64 KiB opaque chunk, credit ACK, threshold 0 и отсутствие compression,
  fragmented/coalesced receives, batch и bundle gates. Отдельно проверены
  malformed/unknown packets, nonfinite pose, относительная первая позиция,
  packet order, несовпавший/повторный batch, отсутствие batch ACK, respawn,
  reconfiguration, transfer, отказ после обязательного pack и отмена.
- Прежние 19 plain login, 13 encrypted login и 12 configuration TCP-сценариев
  прошли с обновлёнными кодеком и транспортом: всего 65 TCP-сценариев этого этапа.
- Успех `PLAY_SESSION_OBSERVED` требует Login, отправленного pose ACK, получения
  chunk, полного подтверждённого batch и ответа на keepalive; открытые bundle/
  batch не допускают преждевременный успех. Позиция и dimension выводятся только
  после terminal acquire, как остальные результаты worker.
- Принимаются только заголовки чанков, остальное остаётся opaque. Registry/light
  не устанавливаются, PlayerLoaded, движение и client ticks не отправляются.
  Respawn/StartConfiguration/Transfer прекращают тест вместо выдуманного продолжения.
- Сохраняются общий post-DNS deadline 30 s, 4096 post-login frames и 64 MiB
  decoded total. Сервер может прислать keepalive примерно через 15 s; успешный
  обмен с fixture не устанавливает расписание настоящего Java-сервера.

Удалены только регенерируемые C object/map-каталоги предыдущих проб (около
10.1 MiB); NRO, ELF и verification records сохранены. Артефакты после новой
сборки около 474 MiB из 512, свободно около 15.7 GiB. Дополнительных зависимостей,
образов и toolchain не загружалось. Аппаратная play-сессия и передача нового NRO
на SD пока не выполнены.

## Live Java 26.3 LAN-мир: host Play Session

2026-11-16 status-запрос к текущему LAN-миру пользователя подтвердил
version.name `26.3`, protocol `777`. Затем host runner с тем же play-кодеком
и C transport успешно достиг `PLAY_SESSION_OBSERVED` (stage 17) за 16.59 s:
Error 0, codec 0, compression 256, encryption true, configuration ACK true.
Получены 32 registry-пакета; первый play ID 50; разобраны Login и позиция,
отправлен 1 pose ACK и 1 keepalive reply. Приняты 49 opaque чанков, подтверждён
41 chunk batch; всего 795 play-пакетов / 1695840 decoded play bytes,
291706 received wire bytes за соединение. Последний play ID 45 (KeepAlive).

Это проверка настоящего Java LAN-мира с компьютера через host OpenSSL backend,
а не выполнение `.nro` на Switch. Она подтверждает совместимость этого
ограниченного wire-обмена с текущим сервером, без PlayerLoaded или движения.
Не заявляется vanilla parity для иных версий/серверов или корректность chunk
schema. Адрес/порт и session UUID в этот результат не включены. Локальный JSON
отчёт сохранён в ignored `tests/build/live-play-session.json`.
Аппаратная Play Session Probe 0.4.0 всё ещё ожидается.

## Передача Play Session Probe 0.4.0 через MTP

2026-11-16 `craftrustnx-play-session-probe.nro` передан через DBI/MTP
в `1: SD Card` → `switch/craftrustnx/`, object ID 16777269.
Обратное чтение подтвердило побайтовое совпадение: 575685 байт, SHA256
`c429b553268dff4395d76e1812b23ab57be6f6ff85021c6ae9e3e8428dd80dc4`.
Прежние NRO не заменялись; временные helper/readback удалены после проверки.
Host-проверка настоящего Java-сервера успешна, аппаратный запуск `.nro` на Switch
всё ещё ожидается.

## Аппаратный результат Play Session Probe 0.4.0

2026-11-16 фотография пользователя подтвердила на Switch Lite:
Stage `PLAY_SESSION_OBSERVED`, Error `NONE`, Encryption active `yes`,
Server should_authenticate `true`, Compression `256`, Configuration ACK `yes`.
Frames `1142`, Received `294818 bytes`, configuration/play packets `1139`,
registry packets `32`, decoded configuration/play bytes `1914988`.
Первый play ID `50`, body `110 bytes`; play packets `1102`, decoded play
bytes `1698326`. Pose ACKs `1`, keepalive `1`, pong `0`, opaque chunks `49`,
batch ACKs `41`; dimension `minecraft:overworld`, game mode `0`.
Локальный адрес/порт, entity ID, положение и session UUID в запись не включены.

Это аппаратное подтверждение структурного Login/позиции, IEEE float/double
обработки и полного pose ACK, зашифрованных play-ответов, получения chunk bodies
и подтверждения batch, а также keepalive на текущем Java 26.3 LAN-мире.
Pong на устройстве в этом запуске не проверен (счётчик 0). Тест завершил
соединение после наблюдения всех необходимых условий. Чанки и registry/light
по-прежнему не разобраны семантически: PlayerLoaded не отправлялся, мира,
физики, управления и рендера ещё нет. Следующий этап — структурный разбор
registry/dimension и содержимого сетевых чанков.

Сохранён переданный NRO 0.4.0 с прежней SHA256
`c429b553268dff4395d76e1812b23ab57be6f6ff85021c6ae9e3e8428dd80dc4`.
Повторной сборки или загрузок ради фиксации результата не выполнялось.

## World Data Probe 0.5.0: host и сборка Switch

2026-11-16 добавлен allocation-free Rust-разбор сетевого NBT, selected dimension/biome
registry, heightmaps, секций, paletted containers, block-entity NBT и light data.
Первый live-host запуск выявил ошибку в исходном задании: маски света были заданы
как big-endian long array. Исправлено по protocol `buf::BitSet` и реальному
26.3 обмену: VarInt byte length + low-byte-first BitSet bytes. Независимый
литеральный regression vector проверяет этот формат. Временный перехваченный
пакет для диагностики удалён; в git остаются только синтетические fixtures.

Проверки: 103 Rust unit tests, fmt и clippy `-D warnings` проходят.
Все 79 TCP-сценариев проходят: 19 login + 13 encrypted login + 12 configuration
+ 21 play + 14 world-data. Новые 14 world-data сценариев также проходят с C
ASan/UBSan (`detect_leaks=0`, macOS); Rust/OpenSSL этой сборкой не инструментированы.
Сценарии покрывают palette modes, bit masks, >64 KiB chunk body, неправильные
heightmap/palette/light данные, отсутствующие registry и устаревший pose sample.
Это независимые synthetic wire fixtures, не полный vanilla parity harness.

Host runner с настоящим Java 26.3 LAN-миром завершился за 16.79 s:
`WORLD_DATA_OBSERVED` (18), Error/codec/world status 0, encryption yes,
configuration ACK yes, 32 registry packets. Сохранены 4 dimension entries и
biome count 67; выбранная высота 384, min Y -64, skylight true. Проверены
49 chunk columns, 1176 sections, 147 heightmaps, 78 block entities,
147 sky layers и 164 block layers. Получен образец raw state ID 126 / biome ID 0
под подтверждённой позицией, отправлены 1 pose ACK, 41 batch ACK и 1 keepalive reply.
Отдельный LightUpdate в этом запуске не получен (0), но покрыт синтетическим
TCP-сценарием. Адрес/порт, положение игрока и session UUID в запись не включены.
Не реализованы block-name mapping, terrain cache, light propagation, renderer
или PlayerLoaded. Результат host OpenSSL не заменяет аппаратный запуск Switch.

`bash scripts/build-switch.sh docker world-data` успешно собрал Rust staticlib
и native NRO без сети/загрузок в существующем образе. Артефакт
`dist/switch/craftrustnx/craftrustnx-world-data-probe.nro`: 587973 bytes, SHA256
`50f22eb400791946cbd22f637efe6e223d113254a41cc319525f7e02563ab9c5`,
NRO0 / ASET / NACP title `CraftRustNX World Data Probe`, version `0.5.0`.
ELF64 LE AArch64 DYN, нет DT_TEXTREL, все 592 dynamic relocations пишут
в writable segments. Прежний NRO 0.4.0 сохранил свою SHA256.

После проверки удалены только регенерируемый host debug target и object-файлы
неудавшегося EGL-аудита. Сохранены NRO/ELF, release archives и отчёты.
Артефакты около 473 MiB из 512; свободно около 15.3 GiB, резерв 10 GiB соблюдён.
MTP на момент подготовки не обнаружен. Передача и аппаратный запуск 0.5.0 ожидаются.

GPU: offline audit подтверждает installed SDK и работу UAM (512-byte DKSH из
оригинального vertex shader). Выполнения GPU-кода на Switch, EGL acceleration
или производительности это не подтверждает. Неоконченные материалы неудавшихся
агентов изолированы в ignored tests/build и не включены в публичный аудит.

## Передача World Data Probe 0.5.0 через MTP

2026-11-16 новый `craftrustnx-world-data-probe.nro` передан через DBI/MTP
в `1: SD Card` → `switch/craftrustnx/`, object ID 16777270. Обратное чтение
подтвердило побайтовое совпадение: 587973 bytes, SHA256
`50f22eb400791946cbd22f637efe6e223d113254a41cc319525f7e02563ab9c5`.
Прежние файлы не заменялись. Временный helper и readback удалены после проверки;
отчёт сохранён в ignored `tests/build/mtp-world-data.json`.
Ожидается аппаратный результат `WORLD_DATA_OBSERVED` / Error NONE; передача
сама по себе не подтверждает запуск нового декодера на Switch.

## Аппаратный результат World Data Probe 0.5.0

2026-11-16 пользователь подтвердил `WORLD_DATA_OBSERVED` на Switch Lite.
Это конечный успешный этап 0.5.0: получены selected registries, структурно
проверен хотя бы один chunk, получен sample под подтверждённой позицией,
отправлены pose/batch ACK и keepalive reply. Фотография и точные аппаратные
счётчики этого запуска не предоставлены; host-значения не выдаются за них.
Проверка не включает terrain cache, light propagation, mesh или renderer;
PlayerLoaded не отправлялся. Следующий отдельный этап — GPU smoke test.

## Rust GPU Probe 0.6.0: подготовка и сборка

2026-11-16 добавлен отдельный GPU smoke test. Rust no_std staticlib задаёт
clear/geometry/color/animation и pause/palette/reset/exit. Libnx обвязка читает
кнопки и время, а C/deko3d мост создаёт double-buffered 1280×720 RGBA8 target,
копирует 96-byte std140 uniforms и отправляет draw. Оригинальные vertex/fragment
GLSL скомпилированы UAM и включены в RomFS; game assets не используются.

11 Rust-тестов, fmt и clippy `-D warnings` прошли при проверке родителем.
Проверены ABI 32-byte state / 128-byte frame, independent geometry/time vectors,
контроли, nonmonotonic/large timestamps, bounds/finite output и null arguments.
Rust cross-build повторно использовал существующий core cache, Cortex-A57/PIC.
C main прошёл SDK syntax check, production main/native objects собраны с
`-fPIE -Wall -Wextra -Werror -Wframe-larger-than=32768`; compile-only SDK check
тоже повторён после интеграционных исправлений (72 KiB artifacts).

Parent review исправил три ошибки в исходном мосте: cleanup после partial init,
shared uniform overwrite до завершения GPU и неограниченное накопление команд.
Сейчас каждый кадр ждёт queue idle перед overwrite, очищает command arena,
заново записывает framebuffer binding/draw и инвалидирует shader/L2 caches.
256-byte uniform range содержит 96-byte Rust payload. Shutdown проверяет каждый
созданный handle независимо от флага полного init. Это осознанная сериализация
smoke test, не доказательство эффективности окончательного renderer.

`bash scripts/build-switch.sh docker gpu` успешно создал
`dist/switch/craftrustnx/craftrustnx-gpu-probe.nro`: 270453 bytes, SHA256
`b72a999422e01e0b77c3d1d12bfd831e82a450097de88bcc43f629027598b4f5`.
NRO0 / ASET / NACP title `CraftRustNX Rust GPU Probe`, version `0.6.0`;
ELF64 LE AArch64 DYN, без DT_TEXTREL, все 404 dynamic relocations в writable
segments. Независимый обход встроенного RomFS подтвердил точное содержимое
`/shaders/probe_vsh.dksh` (768 bytes, DKSH) и `probe_fsh.dksh` (512 bytes, DKSH)
сравнением с выходами UAM. SDK notices приложены отдельным текстовым файлом.

После scoped host-тестов удалён только их регенерируемый debug target;
перед сборкой удалены object-файлы World Data Probe, его NRO/ELF сохранены.
Итоговый experiment budget около 478.5 MiB / 512, свободно около 15.3 GiB.
Никаких toolchain/dependencies/images не загружалось. Неоконченные agent reports
с ошибочными license/synchronization/performance claims исключены из публичных
документов и сохранены только в ignored build/unreviewed-agent-notes.
Аппаратная проверка GPU и передача новой пробы на SD ещё ожидаются.

## Передача Rust GPU Probe 0.6.0 через MTP

2026-11-16 через DBI/MTP в `1: SD Card` → `switch/craftrustnx/` переданы:
`craftrustnx-gpu-probe.nro` (object 16777271, 270453 bytes, SHA256
`b72a999422e01e0b77c3d1d12bfd831e82a450097de88bcc43f629027598b4f5`)
и `craftrustnx-gpu-notices.txt` (object 16777272, 2181 bytes, SHA256
`adee1f5a3fab938a8184f356a77a04ccd3818587b6708902b9dc4aa23dd12ec6`).
Обратное чтение обоих файлов подтвердило побайтовое совпадение. Старые NRO
не заменялись; временный transfer helper/readback удалены. Отчёт сохранён
в ignored `gpu-probe/build/mtp-transfer.json`.
Аппаратный результат GPU пока ожидается: видимая анимация, pause/palette/reset,
выход к summary и возврат в hbmenu проверяются пользователем отдельно от передачи.

## Аппаратный результат Rust GPU Probe 0.6.0

2026-11-16 пользователь подтвердил на Switch Lite: движущийся цветной
треугольник отображается, A/B/X и выход через Plus работают. Это подтверждение
выполнения оригинальных шейдеров и вывода Rust-generated кадров через native
deko3d, изменения uniforms/палитры, паузы/сброса и выхода из пробы.
Фото summary, frame count, GPU allocation bytes и измерения времени кадра
не предоставлены, поэтому численные performance/memory claims не делаются.

Проверен переданный NRO 0.6.0 с SHA256
`b72a999422e01e0b77c3d1d12bfd831e82a450097de88bcc43f629027598b4f5`.
Повторная сборка для фиксации результата не выполнялась. Это standalone
serialized triangle smoke test; Horizon/wgpu, depth/mesh/texture pipeline,
рендер сетевого мира и окончательный backend ещё не проверены.

## Column Snapshot Probe 0.7.0: интеграция

2026-11-17 завершены Rust column copy и интеграция с отдельной сетевой пробой.
Сохранена общая реализация chunk/palette parser: validating pass без записи,
проверка ёмкости, затем copy pass в caller-owned u32 arrays. В snapshot
сохраняется колонка под подтверждённой позицией; образец из массивов сверяется
с world decoder sample. Новая коррекция позы инвалидирует selection. Данные
остаются в памяти после закрытия соединения, но это snapshot принятого пакета,
а не cache с block/light updates. PlayerLoaded не отправляется.

Parent verification: 109 Rust tests, fmt и clippy `-D warnings` проходят.
Дополнительно исправлен overlong palette vector в новом regression test.
93 TCP-сценария проходят: 19 login, 13 encrypted, 12 configuration, 21 play,
14 world-data и 14 column-snapshot. Snapshot runner собран с C ASan/UBSan
(macOS detect_leaks=0), stderr проверяется fixtures. Независимые FNV-style
word digests сверяют все state/biome cells; сами данные реального мира
не сохраняются в git. Rust/OpenSSL не инструментированы этой C сборкой.

Попытка live-host подключения к прежнему endpoint завершилась через 30 s:
stage 9 / timeout 4, encryption 0, chunks 0. Login не достигнут; это не
положительный live результат новой пробы. Запрошены актуальные IP/порт мира.

Новая сборка `bash scripts/build-switch.sh docker column-snapshot` прошла
в существующем образе без загрузок. Большой static job переведён в BSS с
явной инициализацией небольших настроек при старте; это устранило zero-filled
4 MiB arrays из NRO data. Проверяющий инструмент исправлен: SHT_NOBITS/BSS
не читается как file-backed section. Прежние 0.4/0.5/0.6 артефакты также
проходят обновлённую проверку и сохраняют прежние SHA256.

Итоговый `craftrustnx-column-snapshot-probe.nro`: 465093 bytes, SHA256
`c10178bb5623d804e577398f33e6b619fb0ae8947d9d5a46ac2503007e6c3f16`,
NRO0/ASET/NACP title `CraftRustNX Column Snapshot Probe`, version 0.7.0;
ELF64 LE AArch64 DYN, нет DT_TEXTREL; все 596 dynamic relocations в writable
segments. Снимок максимум 256 секций требует 4.0625 MiB статических массивов
в памяти; 384-high column использует 390 KiB полезного output.

После агента experiment budget превысил лимит (515.6 MiB). До дальнейших
сборок удалён только redundant codec/target, затем host debug artifacts после
проверки; исправлены/заменены oversized промежуточные файлы новой 0.7 сборки.
NRO/ELF предыдущих проб и отчёты сохранены. Итог около 485.6 MiB/512,
свободно около 15.5 GiB; toolchain/dependencies/images не загружались.
Передача 0.7.0 по MTP и аппаратная проверка ещё ожидаются.

## Live Java 26.3: Column Snapshot Probe 0.7.0

2026-11-17 после получения актуального endpoint от пользователя host runner
завершился `COLUMN_SNAPSHOT_OBSERVED` (19) за 16.06 s. Error, codec, world и
column status — 0; encryption yes, configuration ACK yes, registry packets 32.
Получены 49 chunks, отправлены 41 batch ACK, 1 pose ACK и 1 keepalive reply.
Сохранён нормализованный snapshot из 24 sections: 98,304 block states и 1,536
biome cells. Образец из полного массива совпал с world sample под
подтверждённой позицией. Word digests: blocks 926093479, biomes 1964077050.
Это fingerprints полученного содержимого, не независимая vanilla parity oracle.

Запуск выполнен с C ASan/UBSan на хосте, stderr пуст; runtime diagnostics
не обнаружены. Rust/OpenSSL этой C сборкой не инструментированы.
JSON с результатом сохранён в ignored `tests/build/live-column-snapshot.json`;
raw column, адрес/порт, положение игрока и session UUID не публикуются.
Полные массивы остаются только в памяти runner и освобождаются с процессом.

Это положительный live-host результат 0.7.0 вместо прежнего timeout.
Аппаратная 0.7.0 проверка на Switch ещё ожидается; MTP не обнаружен.
Проверка не включает live block/light updates, terrain rendering или PlayerLoaded.
Время — длительность диагностического обмена с ожиданием keepalive, не benchmark.

## Передача Column Snapshot Probe 0.7.0 через MTP

2026-11-17 `craftrustnx-column-snapshot-probe.nro` передан через DBI/MTP
в `1: SD Card` → `switch/craftrustnx/`, object ID 16777273.
Обратное чтение подтвердило побайтовое совпадение: 465093 bytes, SHA256
`c10178bb5623d804e577398f33e6b619fb0ae8947d9d5a46ac2503007e6c3f16`.
Прежние NRO не заменялись. Временный transfer helper/readback удалены после
проверки; JSON отчёт сохранён в ignored `tests/build/mtp-column-snapshot.json`.
Аппаратный 0.7.0 результат пока ожидается; host live pass не выдаётся за него.

## Аппаратный результат Column Snapshot Probe 0.7.0

2026-11-17 фотография IMG_0639 от пользователя подтверждает на Switch Lite:
`COLUMN_SNAPSHOT_OBSERVED`, `Error: NONE`, encryption active yes,
server should_authenticate true, compression 256, configuration ACK yes.
Snapshot: 24 sections, 98,304 blocks, 1,536 biomes, status 0;
`Snapshot sample agrees: yes`. Это подтверждает вызов полного Rust column copy
и сверку sample из сохранённых caller-owned массивов на устройстве.

Frames 943, received 423553 bytes; configuration/play packets 940, registries 32,
decoded configuration/play bytes 2299498. Первый play ID 50, body 110 bytes;
play packets 903, decoded play bytes 2082836. Pose ACK 1, keepalive 1, pong 0,
received/checked chunks 49, batch ACK 41. Dimension minecraft:overworld, mode 0.
Dimension entries 4, biome entries 67; min Y -64, height 384, skylight yes.
Checked sections 1176, heightmaps 147; light updates 0, sky layers 174,
block layers 259. На фото raw sample state ID 121, biome ID 3. Адрес/порт,
позиция, entity ID, player/session UUID в эту запись не включены.

Проверен переданный NRO 0.7.0 с SHA256
`c10178bb5623d804e577398f33e6b619fb0ae8947d9d5a46ac2503007e6c3f16`.
Повторная сборка не требовалась. Последующие block/light updates не применяются;
это snapshot приёма, а не live world cache. Terrain renderer, block/model
semantics, сохранённое освещение и PlayerLoaded не реализованы.

Следующий этап требует semantic state tables и моделей из пользовательской
установки pinned Java 26.3. В стандартных локальных папках Minecraft/Prism и
`.local/horizon-client/26.3` ресурсы не найдены; у пользователя запрошено
расположение установки. Никаких game files или prepared resources не добавлено.

## Подготовка моделей/текстур из установленного Minecraft 26.3

2026-11-17 найден клиент OffPrism 26.3; root version.json подтверждает id 26.3,
protocol 777, SHA1 JAR `e877b6a07acd633fb3bb475002175cec036e7b87` совпадает
с Horizon research-lock. Установка используется только для чтения.

Добавлен stdlib Python devtool `tools/stage_switch_assets.py`: строгая проверка
пина, ZIP names/duplicates/symlinks/CRC, limits, atomic staging и manifest SHA256.
Родитель исправил чтение nested minecraft секции настоящего research-lock;
новый вариант проверяется synthetic test вместе с отказом при неверном hash.
Восемь tests, py_compile и diff check проходят. CLI dry-run выполнен до записи.

В ignored `.local/switch-assets/26.3` извлечены 6906 файлов: 2,986,504 data bytes,
manifest 1,287,232 bytes. Независимое сравнение каждого файла с установленным JAR
и recorded SHA256 прошло. Обход block models: 2883 models, 5326 concrete bindings,
18 extra referenced PNGs. Учтены object texture bindings `{sprite, force_translucent}`.
`block/air.json` ссылается на `minecraft:missingno`, которого нет как PNG;
эта missing reference явно сохранена в манифесте, ничего не синтезировано.
Полного atlas/model baking и state-ID mapping этим импортом не выполняется.

До извлечения общий budget был 495.5 MiB; множество мелких файлов требует больше
дисковых блоков, чем сумма их byte lengths. Удалены только регенерируемые
`std-probe/build/vendor` и `std-probe/build/host-target` старой завершённой пробы.
Её NRO/ELF и verification records сохранены; vendor restaging предусмотрен
preflight.sh из уже установленных исходников, без сети. После подготовки
budget около 499.1 MiB/512, free 16.7 GiB. Проверяющий budget script теперь учитывает
оба новых каталога switch-assets/switch-state-facts. Game files/manifest/per-file
hashes не включены в публичный коммит; git check-ignore для output прошёл.
Экспорт semantic state facts выполняется отдельной задачей и ещё не принят
родителем. Новая версия NRO на этом этапе не создавалась.

## Проверенный semantic state export для Switch

2026-11-17 принят devtool `tools/switch_state_facts`: Java registry exporter и
offline Python runner. Родитель заменил исходные opaque property strings на
canonical name/value map через Property.Value.valueName(), проверил contiguous
IDs, уникальность block+property tuples, metadata pin, типы полей и три vanilla
air names. Numeric air IDs не предполагаются. Minecraft JAR совпадает с ранее
проверенным pin 26.3/protocol 777; установка остаётся read-only.

Classpath выбирается из launcher metadata с проверкой SHA1 каждого cached JAR:
43 JARs, без downloads. Исходник компилируется `javac --release 25`, проверяется
class-file major 69; фактический runtime установленного JDK — 27.0.1.0.3.
Это не запуск HotSpot 25 и не доказательство его numeric parity. Компиляция и
registry bootstrap используют scratch cwd/user.home и отдельные Horizon/XDG
homes; injection variables Java удаляются. Game client, server или world не
запускаются. Heap 512 MiB, compile/run timeout 60 s. Bounded writer и проверка
общего output ограничивают экспорт 12 MiB; ошибка не публикует partial output.

Исправленный ignored `.local/switch-state-facts/26.3/state_facts.json` содержит
35,723 states (IDs 0..35722), 3 air и 13,389 nonempty-fluid states.
Размер 9,946,032 bytes, SHA256
`131d7939c39658ac227c711740dc0313e438ba260e93984bee592e07297cfbbd`.
Только устаревший output этой задачи с raw properties заменён после проверки.
Никакие facts, asset manifests, logs или game files в публичный коммит не входят.

Model-link probe проверил наличие blockstate JSON и selected model references
для MODEL states в staged assets. 64 wall states (up=false, все arms=none)
закономерно выбирают ноль multipart parts; их нельзя заменять solid cubes.
Fluids требуют отдельной геометрии. Проверка ссылок не выполняет baking,
texture/UV/lighting parity, mesh generation или rendering.

Три synthetic runner tests проходят: semantic validation/refusal, source pin
и точный выбор cached dependency, failure cleanup и isolated subprocess setup.
`py_compile` и diff check проходят. После интеграции около 498.7 MiB/512 MiB
artifacts и local inputs, свободно 16.7 GiB. NRO/ELF прежних проб сохранены;
новая NRO на этом этапе не создавалась. Перед следующей сборкой требуется
освободить regenerable task outputs до необходимого build headroom.
Public-tree checker допускает только точный путь нашего original Java devtool;
общий запрет recovered Java, JAR/classes и prepared resources сохраняется.
Pre-commit теперь вызывает `--staged`: проверяются все пути index и fingerprints
нового/изменённого staged content. Это устраняет lazy fetch неизменённых blobs
при полном content scan partial clone. Default full-index scan и history/range
checks сохранены. Synthetic Git fixture подтверждает отказ на copied tables и
JAR, а также работу с отсутствующим локально unchanged blob. Итого 12 scoped
tests проходят (8 asset importer, 3 exporter, 1 staged gate).

## Изменение политики места на диске

2026-11-17 пользователь отменил требование экономить место. AGENTS.md и активные
build/preflight scripts больше не навязывают лимит 512 MiB и резерв 10 GiB.
Общий helper сообщает фактический расход и проверяет только явно запрошенный
headroom операции; std preflight сравнивает оценку роста с доступным диском.
Кэши сохраняются, automatic downloads/toolchain installs по-прежнему запрещены.
До этого изменения удалён только регенерируемый Cargo target завершённой std
пробы (около 213 MiB); её NRO, ELF и verification records сохранены. Shell syntax
checks и три exporter tests проходят; сборка старых проб для изменения политики
не требуется. Исторические записи выше отражают действовавшие тогда ограничения.

## Rust model selection: полный pinned resource set

2026-11-17 добавлен standalone host tool `tools/switch_model_prep`: cached
serde_json 1.0.154 и sha2 0.10.9, offline/locked build, без JVM, downloads или
запуска игры. CLI проверяет manifest pin и size/SHA256 всех 6906 listed assets,
canonical state IDs/properties, выбирает ordered variant/multipart parts и
проверяет наличие selected model JSON. Output bounded 12 MiB, атомарная
no-replace публикация через hard link; existing output не заменяется.

Родитель исправил найденный настоящими ресурсами дефект selector: multipart
property alternatives `side|up` должны сравниваться отдельно, с поддержкой
per-term negation; variants используют comma-separated equality clauses,
не выдуманный OR синтаксис. Array/object distinction сохранён как `weighted`:
singleton weighted list расходует RNG draw в upstream, plain object — нет.
Положительные веса и prefix total ограничены signed32. RNG choice tool не делает.

Полный локальный run: 35,723 states, 35,637 MODEL states, 72,572 parts,
77,425 options, 2575 unique model references, 64 valid empty wall selections.
Ignored `.local/switch-models/26.3/model-selection.json`: 7,281,360 bytes,
SHA256 `a623d4ebd56b81e6dc2f8c79aa211779f49f7adc3c9e27f1b78b11f3177ae06d`.
Независимый Python evaluator сравнил каждую часть/альтернативу каждого state,
включая rotation, uvlock, weights и array marker: 35,723/35,723 agree.
Это independent resource-selection check, не vanilla JVM rendering oracle.

20 Rust tests проходят (17 selector, 3 CLI integration), clippy all-targets
`-D warnings`, fmt и shell syntax check clean. Также проходят 12 существующих
scoped Python tests (asset importer/exporter/public gate). Full Horizon build,
server login и новая NRO не требовались; проверенное wire поведение не менялось.

Kiro baking audit принят только после сверки родителем с актуальными исходниками:
добавлена зависимость horizon-assets от horizon-core/libm, исправлены vertex
размеры, неустановленные memory bounds и empty wall semantics. Существующие
Rust ModelLibrary и bake_block_model — кандидаты для следующего host baking
шага. Текущий строгий selector не заявляет полную совместимость с resource packs:
overlap recovery, duplicate-key ordering, coercions и z rotation не реализованы.
Model inheritance, texture resolution/atlas, baked quads, light и terrain renderer
этим планом не создаются. Game/prepared files остаются только в ignored `.local`.

## Horizon Rust baker и Model Mesh Probe 0.8.0

2026-11-17 host helper `tools/switch_model_prep/src/geometry.rs` переиспользует
существующие CuboidModel, ModelLibrary и **bake_block_model** Horizon. Новая
геометрическая математика не писалась. Декодируются только selected models и
parents; отсутствующие/cyclic/слишком глубокие chains, loaders, unresolved face
materials, null elements/faces и non-finite quads дают явный отказ без cube
fallback/partial output. Родитель дополнительно исправил обход depth limit
через уже cached parent и добавил независимый regression case.

Offline/locked dependency resolution с path dependencies horizon-assets и
horizon-core прошло без downloads; использованы cached serde/log/libm. Full
Horizon client/workspace не собирался. Полная host проверка после parent fix:
6906 assets, 2,986,504 bytes; 35,723 states / 77,425 options → 7856 unique requests,
7856 baked results, 79,851 quads, 1247 material sprites. Empty results 35,
tinted quads 573, force-translucent 580, cull metadata 38,944. Render types default.
Это проверка Rust baking resources, не vanilla JVM rendering oracle. Отдельные
block-entity/fluid renderers и корректное отображение пустых special models этим
не доказываются. Bounds host Vec не являются измерением памяти Switch.

Scene exporter выбирает по именам и полной property tuple девять состояний:
stone, oak stairs, cobblestone wall, oak fence, oak slab, grass block, dandelion,
glass и torch. Для статического diagnostic layout берётся явно первый weighted
option каждой части; RNG/parity не заявляется. Получены 12 unique bake requests,
86 quads → 172 triangles → 516 vertices. Binary CTMESH01: 8304 bytes, SHA256
`594e84b5dded66d7e0b0216ebc115b0298823f24b6a6ee587126f271eacfad87`.
Каждая вершина происходит из upstream baked quad + grid translation; новых
solid cubes не синтезировалось. Геометрия private, в Git не включена.

Новая standalone no_std Rust library на Switch делает two-pass transactional
mesh decode, проверяет format/protocol/count/finite coordinates/bounds и строит
right-handed column-major camera MVP с zero-to-one depth. Без allocation,
filesystem или SDK calls. C выполняет file/pad I/O, lifecycle и deko3d SDK calls.
SDK-мост использует RGBA8 color/Z24X8 depth targets, stride16 position/RGBA8
vertices, MVP, queue-idle reuse и command-buffer reset на каждый кадр.

Родитель исправил compile-only SDK реализацию до сборки: 4096-byte memory-block
alignment для uniform/vertex allocations; explicit depth test/write/Less state;
queue wait перед upload; null/non-finite frame argument validation; аппаратную
нормализацию RGBA8 вместо shader bit unpacking; swapchain teardown перед image
backing memory. Shaders встроены в RomFS, Minecraft geometry — отдельный SD файл.
Это serialized diagnostic backend, не результат измерения FPS/throughput.

Проверки: 43 host model-tool tests (33 library, 3 CLI, 7 scene), 6 Rust runtime
tests; clippy all-targets `-D warnings`, fmt и shell syntax clean. 12 существующих
scoped Python tests проходят. Исправлена коллизия временных имён параллельных
test fixtures (clock alone недостаточно; добавлен atomic suffix). C host runner
под ASan/UBSan связан с настоящей Rust staticlib: все 516 vertex records совпали
побайтово, вся сцена внутри начального frustum, controls и отказ corrupt mesh
без partial writes прошли. Rust/зависимости не sanitizer-instrumented.

`bash scripts/build-switch.sh docker terrain` собирает
`craftrustnx-terrain-probe.nro`, 295029 bytes, SHA256
`1809aa952787c4824ec17084d23fac9f2b704a6d279adc739441948c2dba05c4`.
Проверены NRO0/ASET/NACP title **CraftRustNX Rust Model Mesh Probe**, version
0.8.0, ELF64 LE AArch64 DYN, отсутствие DT_TEXTREL и 412 writable runtime
relocations. Старый GPU target 0.6.0 пересобран из изменённого общего Makefile:
SHA256 остался `b72a999422e01e0b77c3d1d12bfd831e82a450097de88bcc43f629027598b4f5`.
Game/launcher/Java/server не запускались, real player data не изменялись.

## MTP-передача Model Mesh Probe 0.8.0

2026-11-17 DBI device 057e:201d: записано только в storage 0x10001,
description **1: SD Card**, каталог `switch/craftrustnx`. Три новых файла:

- `craftrustnx-terrain-probe.nro`: object 16777274, 295029 bytes, SHA256 выше.
- `terrain-probe-mesh.bin`: object 16777275, 8304 bytes, SHA256 выше.
- `craftrustnx-terrain-notices.txt`: object 16777276, 16338 bytes, SHA256
  `31991ba68793f056a539639b33793f465c95f7762e03e450119196695f9fb620`.

Обратное чтение всех трёх файлов подтвердило byte/SHA256 equality. Existing
files не перезаписывались; временные transfer helper/readbacks удалены, ignored
report сохранён в `terrain-probe/build/mtp-verification.json`. NRO/ELF и прежние
проверенные пробы сохранены. Пользователю предложен аппаратный тест девяти фигур,
depth/perspective и A/B/X/right-stick/ZL/ZR/Plus. Аппаратный результат **ожидается**;
host/C-link/packaging/MTP checks не выдаются за device render pass.

## Аппаратный PASS Model Mesh Probe 0.8.0

2026-11-17 пользователь ответил на проверку девяти фигур и A/orbit, B/background,
X/reset, right stick, ZL/ZR/zoom, Plus: **«Фигуры видны, управление работает»**.
Это подтверждает display/control path новой пробы на Switch Lite с переданным
NRO SHA256 `1809aa952787c4824ec17084d23fac9f2b704a6d279adc739441948c2dba05c4`
и mesh SHA256 `594e84b5dded66d7e0b0216ebc115b0298823f24b6a6ee587126f271eacfad87`.
Аппаратная проверка больше не ожидается. FPS, GPU timings, точные memory counters
и отдельная occlusion/precision проверка не сообщались и не заявляются.
Текстуры/alpha, biome/terrain light, live world и gameplay остаются следующими
этапами. Новых билдов или повторной передачи после этого ответа не требовалось.

## Подготовка статической текстурной сцены 0.9

2026-11-17 добавлены Rust atlas builder и отдельный exporter текстурной пробы.
Использованы только ранее подготовленные private assets Minecraft 26.3,
protocol777, SHA1 `e877b6a07acd633fb3bb475002175cec036e7b87` и signatures девяти
моделей пробы 0.8. Все 6906 manifest entries проверяются по size/SHA256 до baking.
Shared host input reader проверяет manifest array schema, pins, paths, case
collisions, symlinks, count/byte limits и hash сцены к manifest. Проверочный
`atlas_check` выводит aggregate JSON без записи ресурсов или отчётов.

12 baked requests дают 86 quads и 10 sprites. Детерминированный статический atlas:
128×128 RGBA8, 65536 bytes, двухпиксельная экструзия краёв/углов. Четыре sprite
содержат alpha<255, частичного alpha нет; шесть quads имеют force_translucent.
Проверены именно исходные baked UV: восемь quads имеют float-погрешность у нуля,
минимум -5.9604645e-8; ни одна не выходит за unit interval более чем на 1e-6.
Exporter clamp-ит только эту погрешность перед atlas mapping; настоящее повторение
UV вне sprite не поддерживается. PNG expansion сохраняет straight source RGBA;
16-bit, APNG и animated mcmeta дают явный отказ без frame-zero fallback.

Private `texture-probe-scene.bin`: CTTEX001, header64, vertex24 (position/UV/RGBA),
три draw ranges по 16 bytes, затем raw atlas. 78032 bytes, SHA256
`0775f880c9ceea0afa9cb7a4bbabf2e68862ceddabff55c97925b5f375ce22b6`.
516 vertices: 396 opaque, 84 cutout, 36 translucent. Независимый Python разбор
подтвердил header, exact length, bounds/finite positions, UV0..1 и все draw ranges.
Нейтральный белый tint; biome/world light, animation, fluids и gameplay не
реализованы. Это отдельная статическая сцена, не live terrain.

Switch runtime остаётся no_std Rust: two-pass transactional scene validation,
прежняя camera пробы 0.8, allocation-free sort целых translucent quads по clip-w
от дальних к ближним. Feature `textured` отключён в старой пробе 0.8. Новые SDK
texture/sampler/blend symbols проверены compile-only в существующем SDK image;
точные API и lifetime constraints записаны в switch-texture-path-audit.md.
Аппаратный texture/alpha/colour PASS пока не заявляется.

## Сборка Texture Probe 0.9.0

Parent integration исправила draft SDK код до принятия: реальные
`DkCopyBuf`/`DkImageView`/`DkImageRect`, sampler/descriptor signatures, blend enable
через ColorState, viewport/scissor/color-write bindings, 64-bit uniform GPU
address, точный source byte count вместо rounded staging size. Удалены
несуществующие image-destroy calls; заменена рекурсивная standalone build схема
на общий probe Makefile. Старый atlas не освобождается до успешного transfer;
image backing и descriptor storage освобождаются после queue idle. Три UBO slots
по 256 bytes предотвращают перезапись mode до чтения GPU. Draw metadata остаётся
в фиксированном CPU массиве. Это serialized diagnostic SDK path.

`bash scripts/build-switch.sh docker texture` проходит в существующем offline
SDK image. NRO: `craftrustnx-texture-probe.nro`, 315765 bytes, SHA256
`a0c0cbaedd16fa1b50d3c5f0a47a9fcd384faf114dc84c163297e412f3b9e538`.
Packaging check: NRO0/ASET/NACP, title **CraftRustNX Rust Texture Probe**, version
0.9.0; ELF64 LE AArch64 DYN, DT_TEXTREL отсутствует, 416 dynamic relocations
попадают в writable segments. RomFS содержит только два original DKSH shaders.
Private scene и SDK/libm notices находятся рядом с NRO; notices SHA256
`31991ba68793f056a539639b33793f465c95f7762e03e450119196695f9fb620`.

Scoped checks: 52 host model-tool tests (41 library, 3 CLI, 7 old scene,
1 textured scene), 8 Rust runtime tests; all-target clippy `-D warnings` и fmt
для обоих standalone crates, shell syntax, строгий SDK C compile и UAM shaders.
Full upstream `pr_ready.py` не запускался: файл отсутствует в sparse checkout;
полная Horizon workspace сборка не входит в эту проверку. Изменений protocol
кодека нет.

Target terrain 0.8.0 также пересобран без feature `textured` и прошёл packaging
check: 295029 bytes, 412 relocations. Новый build SHA256
`8c67e1d66197976186a39f49b495d0d8b98382bd86438ba99e55e067f0c6da75` отличается
от ранее переданного и проверенного на Switch; повторный hardware PASS ему не
приписывается. Ранее переданный SD-файл не менялся. 0.9 на устройство пока не
передавалась и аппаратной проверки textures/alpha/colour не проходила.

## Дополнительная проверка C ABI текстурной сцены

Локально доведены до запуска и проверены сохранённые независимые C fixtures:
`bash platform/switch/texture-probe/tests/run-host.sh`. Исправлена отсутствующая
подкоманда `cargo build`; optional scene reader дополнен проверкой bounds/counts
до allocation/length arithmetic и отказом на truncated read.

Результат: **505 checks, failures=0**. Проверены synthetic little-endian vectors,
unaligned input, invalid header/bounds/UV, incomplete/trailing input, output
capacities, draw gaps/overlap/order, canonical groups, верхние limits и null
arguments; failures zero info и не меняют vertex/draw output. Сортировка сохраняет
opaque/cutout prefix и целые шестивершинные quads; независимые depth keys
32/33/54 подтверждают порядок, включая повторный вызов и отрицательные depths.
Прежние mesh/camera exports также доступны с feature `textured`.

На настоящем private файле проверены 516 vertices, 3 draws, atlas128×128,
побайтовое совпадение decoded vertex/draw records и сохранность всех шести
translucent quads после sort. ASan/UBSan instrumented **только C harness**;
Rust staticlib не инструментирована. Это локальная ABI проверка, не завершённый
делегированный code review и не аппаратный texture/alpha/colour PASS. Изменения
касаются только host tests/документации; новая NRO не требовалась.

## MTP-передача Texture Probe 0.9.0

2026-11-17 DBI 057e:201d (MTP model `Switch`): три новых файла записаны только
в storage0x10001, description **1: SD Card**, каталог `switch/craftrustnx`:

- `craftrustnx-texture-probe.nro`: object16777277, 315765 bytes.
- `texture-probe-scene.bin`: object16777278, 78032 bytes.
- `craftrustnx-texture-notices.txt`: object16777279, 16338 bytes.

Обратное чтение подтвердило byte equality и все три SHA256, приведённые выше.
Existing destination files не перезаписывались, прежняя аппаратно проверенная
0.8 на SD не менялась. Игровые ресурсы остаются private; Git содержит только
код и этот verification record. Ignored report:
`texture-probe/build/mtp-verification.json`. Temporary transfer helper и
readbacks очищены после проверки. Аппаратный результат новой 0.9 ожидается:
девять текстурных фигур, cutout flower, glass/depth, края atlas и управление.

## Аппаратный отзыв 0.9.0 и исправление цветка 0.9.1

2026-11-17 пользователь сообщил: **«все текстуры отображаются нормально кроме
цветка: на текстуре цветка виден z-fightring»**. Это положительный результат
текстурной пробы с отдельным визуальным дефектом цветка; полного PASS без
оговорок и измерения FPS/colour accuracy не заявляется.

Проверена причина: исходный cross model содержит две плоскости, каждая с
отдельными совпадающими front/back quads и противоположным winding. В SDK-мосте
0.9.0 стоял `DkFace_None`: обе стороны обеих quads рисовались в одной плоскости
с разным UV-направлением. В 0.9.1 включены `DkFace_Back` и явный
`DkFrontFace_CCW`, соответствующий outward winding baker. Геометрия, texture
atlas, shader cutout, depth compare и формат данных не менялись; offset/depth
bias для маскировки дефекта не добавлялся.

Независимая проверка настоящего scene binary подтвердила outward CCW normals
всех шести stone faces и две пары совпадающих противоположных flower quads;
для 48 view directions в каждой паре остаётся ровно одна front-facing сторона.
Это геометрическая проверка, не GPU rendering oracle. Strict SDK C compile,
сборка NRO и packaging verification прошли; неизменённые Rust/codec tests
повторно не запускались.

NRO 0.9.1: 315765 bytes, SHA256
`e9383561e6f33958948af5d73865b92b0f3be6033a587a709eddae8ce84c13ce`.
Title **CraftRustNX Rust Texture Probe**, version0.9.1, NRO0/ASET/NACP,
ELF64 LE AArch64 DYN, DT_TEXTREL отсутствует, 416 writable relocations.
Для отдельной передачи подготовлен `craftrustnx-texture-probe-0.9.1.nro`;
прежний переданный NRO 0.9.0 сохранён локально как versioned backup, SD-файл
не менялся. Scene/notices bytes и SHA256 прежние. Аппаратный повторный тест
цветка с разных сторон пока ожидается.

## MTP-передача исправления 0.9.1

2026-11-17 через DBI в storage0x10001 **1: SD Card**, `switch/craftrustnx`,
записан новый `craftrustnx-texture-probe-0.9.1.nro`, object16777280,
315765 bytes, SHA256 `e9383561e6f33958948af5d73865b92b0f3be6033a587a709eddae8ce84c13ce`.
Обратное чтение подтвердило полное совпадение. Existing scene object16777278
и notices object16777279 только прочитаны; их bytes/SHA256 также совпали с
проверенными исходниками. Прежний NRO 0.9.0 не перезаписывался.
Ignored report: `texture-probe/build/mtp-verification-0.9.1.json`.
Temporary helper/readbacks очищены. Аппаратная проверка исправленного цветка
с обеих сторон ожидается; факт передачи не считается visual PASS.

## Аппаратный визуальный PASS Texture Probe 0.9.1

2026-11-19 зафиксирован ответ пользователя после передачи исправления:
**«теперь все модели выглядят правильно»**. Для переданного NRO 0.9.1 SHA256
`e9383561e6f33958948af5d73865b92b0f3be6033a587a709eddae8ce84c13ce`
это подтверждает корректный внешний вид статической сцены и устранение
сообщённого дефекта цветка. Ожидание визуального результата 0.9.1 закрыто.
Отдельные FPS/GPU timings, калибровка цвета и exhaustive alpha/depth проверки
не сообщались. Подключение Java, декодирование колонок и статическая GPU сцена
пока проверены отдельными пробами; живой сетевой terrain renderer этим PASS
не подтверждён. Повторная сборка или передача для фиксации ответа не нужна.

## Column Export Probe 0.10.0: формат, сохранение и сборка

2026-11-24 завершён сохранённый черновик Rust `CTCOL001` и добавлена отдельная
проба `column-export`. Сетевой worker использует прежний путь входа и приёма
колонки. После terminal acquire и join кнопка R сохраняет immutable arrays;
при новом подключении результат сохранения сбрасывается. Rust проверяет точные
счётчики sections*4096/sections*64, min-Y/верхнюю границу, заголовок 26.3/777,
полную длину файла и пределы FFI до работы с данными. C shim отвечает только за
новый каталог, temporary/rename и полное обратное чтение. Существующие capture
directories не заменяются. Light, block entities, credentials и server address
в формат не входят; `.ctcol` остаётся private generated data.

Во время независимого C-прогона найден реальный пропуск в черновике: full decode
не проверял полную длину перед чтением, что давало abort на header-only fixture.
При интеграции исправлены также неточные counts, частичная запись заголовка при
ошибке encode и отсутствие верхнего ограничения FFI length перед созданием
среза. Новая Rust-проверка напрямую закрепляет transactional failure для
неверных размеров. Неиспользуемые endian additions в общем network `wire.rs`
удалены; snapshot helpers локальны модулю. Документированная формула исправлена
на `48 + sections*(4096+64)*4`: 399408 bytes для 24 секций, максимум 4259888.

Проверки выполнены offline, installed Rust1.100.0, cached dependencies, jobs1:

- Rust fmt/clippy all-targets `-D warnings`: PASS; `cargo test`: **132 passed**,
  включая существующие protocol/configuration/play/world/column/NBT проверки.
- `tests/run-snapshot-host.sh`: independent handwritten LE header, complete byte
  order/raw-ID sentinels, negative/extreme coordinates, header-only/full split,
  truncation/trailing, malformed counts/reserved/pins, null/over-limit arguments,
  untouched outputs/tails, 256-section maximum: PASS. Real C-to-Rust staticlib.
- `tests/run-column-store-host.sh`: real temporary files, Rust decode after
  readback, два разных capture directories, первый файл сохраняет исходные
  bytes, invalid input и filesystem-error пути: PASS. Свои temporary files
  удалены. ASan/UBSan применены к C callers/shim; Rust не инструментирован.
- `bash scripts/build-switch.sh docker column-export`: PASS, существующий
  `devkitpro/devkita64:20260219`, `--pull=never --network none`; C SDK checks с
  `-Wall -Wextra -Werror -Wframe-larger-than=32768` отдельно прошли.

`craftrustnx-column-export-probe.nro`: **477381 bytes**, SHA256
`8d2145712e2b56530298504e3be5e5e5a42cf57e623f40c12d70bbbea2a9c0d8`.
`verify_switch_artifact.py` подтвердил NRO0/ASET/NACP, title
**CraftRustNX Column Export Probe**, version **0.10.0**, ELF64 LE AArch64 DYN,
нет DT_TEXTREL; все 597 dynamic relocation targets в writable segments.
Ignored report: `login-probe/tests/build/column-export-artifact.json`.
Артефакты экспериментов 690.8 MiB; свободно 18.3 GiB. Прежние дисковые лимиты
не применялись, compiler caches сохранены.

Это host/cross-build PASS. MTP-передача 0.10.0, сохранение на SD и независимый
decode возвращённого Switch-файла пока не выполнены. Новый live Java-мир в этой
итерации не запускался и не подключался. Прежние проверенные NRO не заменены.
Terrain mesh, освещение, live updates и `PlayerLoaded` не добавлялись.
