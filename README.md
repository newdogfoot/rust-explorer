# Rust Explorer

Rust와 AppKit으로 만든 macOS용 파일 탐색기입니다. Windows 스타일 배치, 한국어 UI, 30개 모양 선택, 탭과 즐겨찾기를 제공합니다.

## 다운로드와 실행

공개 배포본은 [Releases](https://github.com/newdogfoot/rust-explorer/releases)에서 받을 수 있습니다. **Rust-Explorer-macOS-arm64.zip**이 실행 앱이며, Source code ZIP은 개발용 소스입니다.

1. 실행 ZIP을 압축 해제합니다.
2. `Rust Explorer.app`을 응용 프로그램 폴더로 옮깁니다.
3. 앱을 엽니다. 실행에는 Rust나 개발 도구가 필요하지 않습니다.

Apple Silicon(M1 이상)용 테스트 버전입니다. 최소 OS 설정은 macOS 14이며 실제 검수는 macOS 27에서 수행했습니다. 다른 기기와 이전 macOS에서의 실기 호환성 검수는 별도로 필요합니다. Intel Mac용 바이너리는 포함하지 않습니다.

개인용 코드 서명을 사용하며 Apple Developer ID 서명·공증은 받지 않았습니다. 첫 실행이 차단될 수 있습니다. 출처와 파일을 신뢰하는 경우에만 [Apple 공식 안내](https://support.apple.com/102445)에 따라 개인정보 보호 및 보안의 확인 없이 열기 절차를 사용하세요.

## 기능

- 환경 설정에서 30개 모양을 선택하면 모든 창에 즉시 적용·저장합니다. 시스템 모드는 밝게/어둡게 자동 전환합니다.
- 상세 목록과 세 가지 아이콘 크기, 파일 이름 두 줄 표시와 전체 이름 툴팁, 선택과 스크롤 복원.
- 탭·탐색 기록, 사용자 지정 즐겨찾기와 위치, 내부·외장 저장장치, 서버 연결.
- 현재 목록의 이름 필터와 하위 폴더 이름 검색. 심볼릭 링크와 앱 패키지를 자동 재귀하지 않습니다.
- 복사·이동·이름 변경·새 문서·휴지통·충돌 처리·작업 기록과 검증 가능한 되돌리기.
- ZIP/7z 압축, ZIP/7z/RAR/TAR 계열 해제. 암호가 있는 압축파일은 지원하지 않습니다.
- Quick Look, macOS의 AirDrop·Bluetooth 파일 교환 연결.

Windows 프로필의 이름 변경은 F2 또는 Ctrl/⌘Enter입니다. 키보드가 F2를 시스템 키로 전달할 때는 추가 권한 없이 Ctrl/⌘Enter를 사용할 수 있습니다.

## 데이터와 공개 범위

[개인정보와 로컬 저장 데이터](docs/PRIVACY.md), [검수 범위](docs/VALIDATION.md)를 참고하세요. 공개 소스는 `release/public-files.txt`의 파일만 포함합니다. 내부 작업 기록·원본 검수 로그·개발 환경 사진·서명 개인키·로컬 설정 DB를 배포하지 않습니다.

## 개발 빌드

macOS Command Line Tools와 Python 3.11 이상, Git 체크아웃이 필요합니다. 소스 ZIP에는 Git 인덱스가 없으므로 아래 clone 방식으로 빌드하세요. 최초 의존성 fetch와 서명 도구 준비는 인터넷을 사용합니다. 동결된 앱 소스의 컴파일은 네트워크를 차단합니다. Rust 도구체인은 프로젝트 내부에 설치합니다.

```sh
export RUST_EXPLORER_PYTHON=python3.11
git clone https://github.com/newdogfoot/rust-explorer.git
cd rust-explorer
sh scripts/bootstrap.sh
sh scripts/cargo.sh fetch --locked
sh scripts/cargo.sh test --offline
sh scripts/cargo.sh clippy --offline --all-targets --features native-ui-tests -- -D warnings
sh scripts/cargo.sh test --release --offline --features native-ui-tests --test native_selection
sh scripts/cargo.sh test --release --offline --features native-ui-tests --test native_themes
$RUST_EXPLORER_PYTHON scripts/test-privacy-guard.py
sh scripts/bundle.sh
```

번들은 검토된 소스의 동결 사본에서 빌드합니다. 빌드가 출력한 새 후보 디렉터리만 포장 대상으로 사용합니다.

```sh
$RUST_EXPLORER_PYTHON scripts/package.py --candidate dist/candidates/CANDIDATE --expected-inputs dist/reviews/REVIEW.json
```

개인 환경에서 공개하면 안 되는 문자열은 저장소 밖의 JSON 파일에 `forbidden_values` 배열로 넣고 `RUST_EXPLORER_PRIVATE_RULES` 또는 `--private-rules`로 지정할 수 있습니다. 이 파일을 커밋하거나 배포하지 마세요. 배포 검사를 통과한 새 후보의 `publish` 디렉터리에만 ZIP과 검증 기록이 생성됩니다. 이전 dist 파일을 자동으로 재사용하지 않습니다.

서명용 개인키는 프로젝트 밖의 사용자 Application Support에 생성되며 소스·앱·ZIP에 포함하지 않습니다. 동일한 로컬 앱을 업데이트할 때는 기존 키를 보존해야 합니다. 사용자 키체인, 시스템 신뢰 설정, 전역 키보드·마우스 설정을 변경하지 않습니다.
