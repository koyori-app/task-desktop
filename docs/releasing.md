# Desktop のビルドと配布

`desktop/packaging/package.py` はネイティブ実行ファイルから Windows MSIX、macOS DMG、
Linux AppImage と tar.gz を生成します。既定では署名が必須です。署名情報が足りなければ
処理を停止します。スクリプトはインストール、OS 登録、配布先へのアップロードを行いません。
macOS の署名モードは Apple の notarization サービスへ成果物を送信します。

## 共通

各 OS の実機またはビルドランナーで実行します。Python 3.11 以上と
`desktop/rust-toolchain.toml` の Rust が必要です。実行ファイルは各 OS の x64 / ARM64 に対応し、
アーキテクチャはバイナリのヘッダーから検証します。macOS Universal Binary は別途統合せず、
アーキテクチャごとに配布します。

```sh
cd desktop
cargo build --locked --release -p app
python packaging/package.py --self-test
python packaging/package.py --version 1.0.0
```

`--binary` でビルド済み実行ファイル、`--out` で新しい出力ディレクトリを指定できます。
既定の出力先は `desktop/target/packages/<version>-<os>-<arch>/` です。上書きや削除は行わず、
同じディレクトリが存在すれば停止します。`_staging/` はパッケージ内容の確認用、
`SHA256SUMS` は配布ファイルのチェックサムです。

`--unsigned` は署名・notarization のないローカル梱包テスト専用です。
成果物の名前に `-unsigned` を付け、Windows の更新フィードは生成しません。
公開配布には使用しないでください。CI のテンプレート検証は署名情報も梱包ツールも使用しません。

## Windows

Visual Studio Build Tools の MSVC、Windows 10/11 SDK の MakeAppx / SignTool と
Windows PowerShell が必要です。パッケージの最低 OS は Windows 10 2004 です。
Windows SDK ツールが PATH にない場合は `KOYORI_MAKEAPPX` / `KOYORI_SIGNTOOL` に
実行ファイルの絶対パスを設定します。

次の環境変数に、配布担当者が管理する実際の値を設定してください。スクリプトが証明書を
作成・インポートしたり、架空の発行者・配布 URL を使用したりすることはありません。

| 変数 | 値 |
|---|---|
| `KOYORI_WINDOWS_PUBLISHER` | コード署名証明書の Subject と完全一致する発行者 DN |
| `KOYORI_WINDOWS_PUBLISHER_NAME` | インストーラーに表示する発行者名 |
| `KOYORI_WINDOWS_RUNTIME_DIR` | 対象アーキテクチャの Microsoft Visual C++ Redistributable DLL ディレクトリ |
| `KOYORI_WINDOWS_CERT_THUMBPRINT` | CurrentUser の証明書ストアに準備済みのコード署名証明書の SHA-1 thumbprint |
| `KOYORI_WINDOWS_TIMESTAMP_URL` | 署名事業者の RFC 3161 timestamp サービス URL |
| `KOYORI_UPDATE_BASE_URL` | MSIX と appinstaller を配信する HTTPS ディレクトリ URL |

Runtime ディレクトリは通常、VS の `VC/Redist/MSVC/<version>/<arch>/Microsoft.VC143.CRT` です。
SDK のデバッグ DLL は使いません。`vcruntime140.dll` のアーキテクチャを確認し、その
ディレクトリの再配布可能 DLL を同梱します。再配布条件は VS のライセンスに従います。

実行ファイルと MSIX を SHA-256 で署名し、MSIX の署名を `signtool verify /pa` で検証します。
`AppxManifest.xml` は `app.koyori.desktop` / Application Id `Koyori` の full-trust アプリを登録します。
通知にはインストール後の実際の AUMID（PackageFamilyName + `!Koyori`）を使用します。
未登録の単体 exe では OS 通知が表示されない場合があります。

生成される `Koyori-<arch>.appinstaller` は起動時に 4 時間間隔、およびバックグラウンドで
更新を確認します。配布時は先に署名済み MSIX をアップロードし、最後に同じ安定 URL の
appinstaller を差し替えてください。発行者と Identity Name を維持し、Version を増加させます。
初回インストールは appinstaller から行います。更新ファイルは公開する前にステージングの
配信先と別の Windows ユーザー環境で確認してください。

## macOS

Xcode と Command Line Tools、Developer ID Application 証明書が必要です。
`gpui-kit` は `runtime_shaders` を有効にしているため、Metal の事前コンパイル用ツールを
追加ダウンロードする必要はありません。

| 変数 | 値 |
|---|---|
| `KOYORI_MACOS_SIGN_IDENTITY` | Keychain に準備済みの Developer ID Application 署名 Identity |
| `KOYORI_MACOS_NOTARY_PROFILE` | `xcrun notarytool store-credentials` で事前に作成した Keychain profile 名 |

`.app` の識別子は `app.koyori.desktop`、最低 OS は macOS 12 です。ビルド時も
`MACOSX_DEPLOYMENT_TARGET=12.0` を設定し、配布対象の OS で起動確認してください。
スクリプトは Homebrew 等の外部 dylib への依存を検出すると停止します。

スクリプトは `.app` を hardened runtime で署名し、Apple に notarization を要求して
チケットを staple します。その後、Applications へのリンクを含む DMG を生成し、
DMG も署名・notarization・staple・検証します。認証情報は Keychain profile から読み、
CLI 引数や設定ファイルへパスワードを書き込みません。自動更新機構は Phase 2 です。

## Linux

CI は Ubuntu 24.04 を使用します。必要なネイティブ依存パッケージは
`.github/workflows/desktop-ci.yml` の Linux ステップで管理します。
GPUI は X11/Wayland と Vulkan を使用し、トレイは KSNI の D-Bus worker を使用します。
GTK のイベントループは使用しません。実行環境には通知サービス、Secret Service、
およびトレイを使用する場合は StatusNotifierHost が必要です。

配布先の最も古い glibc を持つ環境でビルドしてください。AppImage は glibc や GPU ドライバーの
互換性を解消するものではありません。Ubuntu 24.04 の CI 成功だけで古いディストリビューションへの
対応を宣言しないでください。

次のツールは配布担当者が固定バージョンとチェックサムを確認して用意します。
スクリプト自身は実行コードや AppImage runtime をダウンロードしません。

| 変数 / ツール | 値 |
|---|---|
| `linuxdeploy` または `KOYORI_LINUXDEPLOY` | 依存共有ライブラリを AppDir に集める実行ファイル |
| `appimagetool` または `KOYORI_APPIMAGETOOL` | AppImage を作成する実行ファイル |
| `KOYORI_APPIMAGE_RUNTIME` | 対象アーキテクチャの検証済み type2-runtime ファイル |
| `gpg` | GnuPG、署名鍵は事前準備する |
| `KOYORI_GPG_KEY` | 署名に使う既存鍵の fingerprint |

AppImage には GPG 署名を埋め込み、AppImage と tar.gz の両方に detached signature (`.asc`) を付けます。
tar.gz には AppDir と依存ライブラリが入り、展開後の `Koyori/AppRun` から起動します。
GPU ドライバー、D-Bus サービスなどのホスト依存機能は別環境で操作確認が必要です。
自動更新機構は Phase 2 です。

## 配布前の確認と検証範囲

CI は Windows / macOS / Linux で fmt・clippy・unit test・ネイティブビルドと
パッケージテンプレートの整合性を検証します。署名鍵を持たず、署名済みインストーラーを
生成・公開しません。CI のビルド成功は GUI の実機操作確認や署名済み配布物の検証とは別です。

配布担当者は署名済み成果物について、初回起動・ブラウザ認証・Credential 保存・トレイ常駐・
通知クリック・ログイン時起動・アンインストールを各 OS で確認します。
Windows では appinstaller 経由の更新、macOS では Gatekeeper と notarization、Linux では
X11 / Wayland とトレイのない環境も確認します。

参照: [Microsoft MSIX manifest](https://learn.microsoft.com/en-us/windows/msix/desktop/desktop-to-uwp-manual-conversion)、
[App Installer updates](https://learn.microsoft.com/en-us/windows/msix/app-installer/how-to-create-appinstaller-file)、
[Apple notarization](https://developer.apple.com/documentation/security/customizing-the-notarization-workflow)、
[AppImage signing](https://docs.appimage.org/packaging-guide/optional/signatures.html)、
[linuxdeploy](https://github.com/linuxdeploy/linuxdeploy)、
[appimagetool](https://github.com/AppImage/appimagetool)。
