#!/bin/sh
set -eu

REPO="ridloabelian/saif-agent"
INSTALL_DIR="${SAIF_INSTALL_DIR:-$HOME/.local/bin}"
VERSION="${SAIF_VERSION:-latest}"

OS=$(uname -s)
ARCH=$(uname -m)

case "$OS" in
  Linux)
    case "$ARCH" in
      x86_64|amd64) TARGET="x86_64-unknown-linux-gnu" ;;
      aarch64|arm64) TARGET="aarch64-unknown-linux-gnu" ;;
      *)
        echo "Unsupported Linux architecture: $ARCH" >&2
        exit 1
        ;;
    esac
    ;;
  Darwin)
    case "$ARCH" in
      arm64|aarch64) TARGET="aarch64-apple-darwin" ;;
      x86_64)
        echo "macOS x86_64: prebuilt binaries target Apple Silicon (aarch64-apple-darwin). Build from source with 'cargo install' or 'cargo build --release --bins'." >&2
        exit 1
        ;;
      *)
        echo "Unsupported macOS architecture: $ARCH" >&2
        exit 1
        ;;
    esac
    ;;
  *)
    echo "Unsupported OS '$OS'. On Windows, run install.ps1 instead." >&2
    exit 1
    ;;
esac

if [ "$VERSION" = "latest" ]; then
  BASE_URL="https://github.com/${REPO}/releases/latest/download"
else
  BASE_URL="https://github.com/${REPO}/releases/download/${VERSION}"
fi

ARCHIVE="saif-${TARGET}.tar.gz"
TMP_DIR=$(mktemp -d)
trap 'rm -rf "$TMP_DIR"' EXIT INT TERM

echo "Downloading ${ARCHIVE} (${VERSION})..."
curl -fsSL "${BASE_URL}/${ARCHIVE}" -o "${TMP_DIR}/${ARCHIVE}"
curl -fsSL "${BASE_URL}/SHA256SUMS.txt" -o "${TMP_DIR}/SHA256SUMS.txt"

EXPECTED=$(awk -v f="$ARCHIVE" '$2 == f || $2 == "*"f { print $1; exit }' "${TMP_DIR}/SHA256SUMS.txt")
if [ -z "$EXPECTED" ]; then
  echo "Checksum entry for ${ARCHIVE} not found in SHA256SUMS.txt" >&2
  exit 1
fi

if command -v sha256sum >/dev/null 2>&1; then
  ACTUAL=$(sha256sum "${TMP_DIR}/${ARCHIVE}" | awk '{print $1}')
elif command -v shasum >/dev/null 2>&1; then
  ACTUAL=$(shasum -a 256 "${TMP_DIR}/${ARCHIVE}" | awk '{print $1}')
else
  echo "Neither sha256sum nor shasum found to verify download" >&2
  exit 1
fi

if [ "$ACTUAL" != "$EXPECTED" ]; then
  echo "SHA-256 checksum mismatch for ${ARCHIVE}: expected ${EXPECTED}, got ${ACTUAL}" >&2
  exit 1
fi
echo "SHA-256 verified (${ACTUAL})"

mkdir -p "${TMP_DIR}/unpack" "$INSTALL_DIR"
tar -xzf "${TMP_DIR}/${ARCHIVE}" -C "${TMP_DIR}/unpack"
install -m 0755 "${TMP_DIR}/unpack/saif" "${INSTALL_DIR}/saif"
install -m 0755 "${TMP_DIR}/unpack/saif-acp" "${INSTALL_DIR}/saif-acp"

mkdir -p "${TMP_DIR}/smoke"
VER=$(SAIF_HOME="${TMP_DIR}/smoke" "${INSTALL_DIR}/saif" --version)
echo "Installed: ${VER} -> ${INSTALL_DIR}/saif & ${INSTALL_DIR}/saif-acp"

case ":${PATH}:" in
  *":${INSTALL_DIR}:"*) ;;
  *)
    echo "Add ${INSTALL_DIR} to your PATH:"
    echo "  export PATH=\"${INSTALL_DIR}:\$PATH\""
    ;;
esac
