#!/usr/bin/env bash
# Скрипт сборки PhotoCraft под Android (ARM64 + опционально ARMv7) с использованием cargo-ndk и Gradle.
#
# Использование:
#   ./packaging/android/build-android.sh                # Сборка .so (arm64-v8a) + debug/release APK при наличии gradlew
#   ./packaging/android/build-android.sh --all-abis     # Сборка arm64-v8a + armeabi-v7a
#   ./packaging/android/build-android.sh --so-only      # Только компиляция Rust .so в jniLibs/
#   ./packaging/android/build-android.sh --bundle       # Сборка Android App Bundle (.aab)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
JNI_LIBS_DIR="${SCRIPT_DIR}/app/src/main/jniLibs"

BUILD_ALL_ABIS=0
SO_ONLY=0
GRADLE_TASK="assembleRelease"

for arg in "$@"; do
    case "${arg}" in
        --all-abis) BUILD_ALL_ABIS=1 ;;
        --so-only) SO_ONLY=1 ;;
        --debug) GRADLE_TASK="assembleDebug" ;;
        --bundle) GRADLE_TASK="bundleRelease" ;;
        *)
            echo "Неизвестный аргумент: ${arg}" >&2
            echo "Допустимые флаги: --all-abis, --so-only, --debug, --bundle" >&2
            exit 1
            ;;
    esac
done

# Проверяем наличие cargo-ndk и Android NDK.
if ! command -v cargo-ndk >/dev/null 2>&1; then
    echo "Ошибка: не найден cargo-ndk. Установите его командой:" >&2
    echo "  cargo install cargo-ndk" >&2
    exit 1
fi

if [[ -z "${ANDROID_NDK_HOME:-}" ]]; then
    echo "Ошибка: переменная окружения ANDROID_NDK_HOME не задана." >&2
    echo "Укажите путь к Android NDK r26+ (например: export ANDROID_NDK_HOME=\$ANDROID_HOME/ndk/27.2.12479018)." >&2
    exit 1
fi

TARGETS=("arm64-v8a")
RUST_TARGETS=("aarch64-linux-android")
if [[ "${BUILD_ALL_ABIS}" -eq 1 ]]; then
    TARGETS+=("armeabi-v7a")
    RUST_TARGETS+=("armv7-linux-androideabi")
fi

# Проверяем, что необходимые rustup-таргеты установлены.
for target in "${RUST_TARGETS[@]}"; do
    if ! rustup target list --installed | grep -qx "${target}"; then
        echo "Устанавливаем Rust-таргет ${target}..."
        rustup target add "${target}"
    fi
done

mkdir -p "${JNI_LIBS_DIR}"

NDK_ARGS=()
for abi in "${TARGETS[@]}"; do
    NDK_ARGS+=(-t "${abi}")
done

echo "==> Сборка libphotocraft_android.so (профиль android-release, ABI: ${TARGETS[*]})..."
(
    cd "${REPO_ROOT}"
    cargo ndk "${NDK_ARGS[@]}" --platform 26 -o "${JNI_LIBS_DIR}" \
        build -p photocraft-android --profile android-release
)

echo "==> Нативные библиотеки собраны в ${JNI_LIBS_DIR}:"
find "${JNI_LIBS_DIR}" -name '*.so' -print

if [[ "${SO_ONLY}" -eq 1 ]]; then
    exit 0
fi

if [[ -x "${SCRIPT_DIR}/gradlew" ]]; then
    echo "==> Запуск Gradle (${GRADLE_TASK})..."
    (
        cd "${SCRIPT_DIR}"
        ./gradlew "${GRADLE_TASK}"
    )
elif command -v gradle >/dev/null 2>&1; then
    echo "==> Запуск системного gradle (${GRADLE_TASK})..."
    (
        cd "${SCRIPT_DIR}"
        gradle "${GRADLE_TASK}"
    )
else
    echo "Примечание: gradlew/gradle не найден — пропуск упаковки APK/AAB (библиотеки .so уже готовы в jniLibs/)."
fi
