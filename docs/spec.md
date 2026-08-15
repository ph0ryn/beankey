# NixOS向けZenzai対応かな漢字変換システム 仕様書

- 文書状態: 実装・正常動作確認済み
- 作成日: 2026-08-14
- 関連文書: [architecture.md](./architecture.md)
- 原作スナップショット: [`93766c46e31fa6a18b7ced49dab31337780f6f45`](https://github.com/azooKey/AzooKeyKanaKanjiConverter/commit/93766c46e31fa6a18b7ced49dab31337780f6f45)

## この文書の責任

この文書は、本システムが何を提供するかを定義する。目的、対象範囲、対象外、原作との関係、外部から観測できる要件、完了条件、未決事項を扱う。

採用する言語、コンポーネントの分割、プロセス境界、通信方式などの実現方法は`architecture.md`の責任とし、この文書では必要な製品要件だけを定義する。

## 位置付け

本プロジェクトは、Fcitx5アドオン版azooKeyとして、固定したAzooKeyKanaKanjiConverterで確認できるかな漢字変換機能をSwiftへ依存せずにNixOS向けへ再実装し、日本語入力時の観測可能な操作感を公式azooKey Desktopへ合わせる。

機能は1個ずつ実装し、最終的には固定したconverterスナップショットで確認できるかな漢字変換機能をすべて対象にする。入力状態、キー操作、プリエディット、候補および予測の提示規則は、公式azooKey Desktopの固定スナップショットを参照する。Swift APIのソース互換またはABI互換、macOS固有の描画、設定画面およびアプリケーション機能は対象にしない。

現段階では、実装した各機能がNixOSとFcitx5上で正常に動作することを合格基準とする。固定原作との候補列、内部traceおよびlogitの厳密比較は、機能実装後の適合検証として分離する。

MVPやv1などの段階的な成果範囲は設けない。確定した実装対象と完了条件の全体を、本プロジェクトの一つの成果範囲として扱う。

変換エンジンの参照元はAzooKeyKanaKanjiConverterだけとする。他のLinux向け移植は、比較、設計判断、コード再利用、テストデータ取得のいずれでも意図的に参照しない。Fcitx5との接続部分だけは、Fcitx5でMozcを利用する公式実装である[fcitx5-mozc](https://github.com/fcitx/mozc/tree/3f8dea4bdf72c6af200ecdbe3d456871fb1d5e03/src/unix/fcitx5)に準拠する。

## 確定した方針

- NixOS向けに新しく実装する。
- かな漢字変換エンジンと、公式azooKey Desktopの日本語入力操作をFcitx5標準UIへ写像した入力体験を対象にする。
- Swiftを製品のビルド時にも実行時にも使用しない。
- 固定原作で確認できるかな漢字変換機能を1個ずつ実装し、最終的にはすべてを対象にする。
- 現段階では正常動作を検証し、固定原作との厳密比較は後続作業とする。
- Fcitx5から利用できるようにする。
- Fcitx5との接続、キーconsumed判定および標準UIへの反映はfcitx5-mozcに準拠する。
- 入力中、先頭候補preview中、候補選択中を区別し、Space、Shift+Space、矢印、Escape、Enter、Tab、数字およびF6からF10の状態遷移を公式azooKey Desktopへ合わせる。
- Zenzai推論にはllama.cppを使用する。
- 設定GUIは作らず、NixOS設定を設定の情報源にする。
- NixOS moduleは`programs.beankey`を公開し、対応する唯一のZenzaiモデルをflake内の`pkgs.fetchurl { url; hash; }`で固定して取得する。
- 利用者向け設定はすべて`programs.beankey`配下へ集約する。

## 対象範囲

本プロジェクトの成立に必須とする範囲は次のとおりである。

- NixOS
- Fcitx5から利用する日本語入力
- UIから独立したSwift非依存のかな漢字変換エンジン
- 入力状態の更新、辞書変換、候補提示、候補選択、確定
- 複数のFcitx5入力コンテキストに対応できる独立した変換状態
- llama.cppを使うZenzai候補評価
- 固定原作のconverter libraryで確認できる公開かな漢字変換機能
- `programs.beankey`によるプログラム、辞書、デーモンおよび対応モデルの導入
- 実際のアプリケーション上での動作確認

この「公開かな漢字変換機能」には、通常の全文・文節・単語変換、直接入力、ローマ字、AZIK、かなJIS/US、カスタム入力表、挿入・削除・カーソル移動、入力中予測、確定後予測、英語・ギリシャ語予測、学習と忘却、ユーザ辞書、特殊候補、表記候補、テンプレート候補、Zenzaiの通常・rich・personalization・次入力予測、LM誤入力訂正、ライブ変換、部分確定、再変換および複数入力セッションを含む。これらを依存関係に沿って1機能ずつ追加し、途中状態をMVPやv1として別の完成範囲にはしない。

## 対象外

本プロジェクトでは、次を対象外にする。

- azooKey Desktopの設定画面、独自候補ウィンドウ、置換提案、Unicode入力など、Fcitx5日本語入力の中核に含まれないアプリケーション機能
- Swiftの公開APIとのソース互換またはABI互換
- macOS向け入力メソッド
- 原作CLI、辞書生成、N-gram学習および評価など、converter libraryを利用または支援する開発用command
- 設定GUIおよび独自の候補ウィンドウ
- Swiftツールチェーンの製品依存としての維持
- 原作にない変換精度改善
- 他のIMEとの精度比較
- 他のLinuxディストリビューションの正式サポート
- 汎用LLM実行基盤または複数推論エンジンを交換するための仕組み

## 原作の位置付けと将来比較

原作コード、テストおよびfixtureは実装の根拠として利用するが、現段階の完了条件は固定原作との完全一致ではなく、各機能の正常動作とする。以下の厳密比較は、全機能の実装後に行う適合検証の設計として保持する。

### 比較対象

同等性の基準は、原作スナップショットと、それが固定する辞書サブモジュールおよび本プロジェクトが固定するGGUFモデルである。製品では、`flake.lock`が固定するnixpkgsの`pkgs.llama-cpp`を利用し、原作のllama.cpp `b4846`と同じsource revisionは要求しない。比較時には、双方のllama.cpp revisionを記録したうえで、入力操作列、変換オプション、辞書、モデル、周辺文脈、セッション、キャッシュ、学習状態を固定する。

将来の適合検証では、少なくとも次を比較する。

- 各入力、削除、カーソル移動、確定後の入力履歴、表示文字列、カーソル、残入力
- 候補の文字列、順序、入力の消費範囲、予測や誤入力訂正などの属性
- 候補選択、部分確定、再変換後の状態
- 参照した辞書要素、接続ID、意味ID、コストの符号・加算順・採用経路
- Zenzaiのprompt bytes、token ID、周辺文脈、各評価ループのdraft、修正prefix、制約付き再探索、最終候補

top-1候補の一致だけで、原作と同等とは判定しない。候補列と状態遷移を比較する。ただし、この判定は現段階の機能実装を完了とするための条件にはしない。

### 数値差の扱い

原作はプラットフォームにより辞書スコアへFloat16またはFloat32を使い、llama.cppのrevision、backend、thread数でもlogitが変わり得る。そのため、一つの原作実行条件と、一つのNixOS実行条件をそれぞれ固定する。

入力状態、辞書形式、コストの符号と加算項、prompt bytes、token ID、prefix再探索の差を、浮動小数点誤差として許容してはならない。logitなどの数値だけは、同じargmaxと最終候補を維持する許容差を別途決める。

### 原作テストの位置付け

原作のSwiftテストとfixtureは、再実装の期待値を得るためのオラクルとして利用できる。Swiftテストの実行環境は原作の期待値を記録するためだけに使い、製品のビルドおよび実行時依存には含めない。原作fixtureは製品または本プロジェクトの配布物へコピーせず、正常動作テストには本プロジェクトで作成したfixtureを使用する。

今回の調査では、llama.cpp `b4846`のXCFramework取得時にmacOS Keychainエラー`status -128`が発生したため、`swift test`は完走していない。現時点の調査結果はソースとfixtureの静的確認に基づく。実行結果を受け入れ証拠として扱うには、別途オラクル環境を再現する必要がある。

## 要件

### Swift非依存

本システムは、製品のビルド時と実行時のいずれでもSwiftコンパイラ、Swift標準ライブラリ、Swift製の実行バイナリを必要としてはならない。

原作オラクルの生成を目的として、製品とは分離した環境でSwift版を実行することは許容する。

### NixOS上での利用

本システムはNixOSへ宣言的に導入でき、Fcitx5の入力メソッドとして実際のアプリケーションから利用できなければならない。

NixOS moduleは`programs.beankey.enable`を公開する。今後、利用者が変更する必要のある設定が確定した場合も、すべて`programs.beankey = { ... };`配下へ追加し、手書きのdaemon設定、設定用環境変数または別namespaceは公開しない。モデルはflakeが固定して導入し、利用者向けoptionにしない。

現在の対応・受け入れ確認環境は`x86_64-linux`、NixOS 26.11、Fcitx5 5.1.21、X11、GTK 4アプリケーションとする。X11の確認には隔離したXvfb displayとZenityを使用した。Waylandおよび他のsystem architectureは、package出力が存在する場合も受け入れ確認済みとは扱わない。

### かな漢字変換エンジン

本システムの中核は、UIから独立したかな漢字変換エンジンでなければならない。

原作の入力履歴と表示文字列が一対一ではないこと、入力位置と表示位置が異なること、辞書探索が入力と表示の二つのindexを持つことを維持しなければならない。実装した入力方式、変換、候補および確定機能は、代表入力で破綻せず、状態遷移、候補選択および確定を正常に完了できなければならない。

### 辞書

固定辞書について、原作と同じ語彙、読み、左右接続ID、意味ID、基礎値、動的補正、接続コスト、意味コストを解釈しなければならない。

辞書には`data/azooKey_dictionary_storage` submodule内の固定済み生成物を直接使用する。本プロジェクトへ辞書生成器または別形式への変換処理を含めない。

### Zenzaiとllama.cpp

Zenzai推論には、`flake.lock`が固定するnixpkgsの`pkgs.llama-cpp`を使用する。別の推論エンジンへ交換するための汎用抽象化を要件にしない。

Zenzaiは単純な候補リランカーとして実装してはならない。原作と同様に、辞書候補のdraftをモデルで評価し、モデルが要求したUTF-8 prefixを制約として辞書ラティスへ戻し、再探索と再評価を繰り返さなければならない。

対応するGGUFモデルは、[Miwa-Keita/zenz-v3.2-small-gguf](https://huggingface.co/Miwa-Keita/zenz-v3.2-small-gguf)のcommit `c67e03e07d215c869f591b274c1631170d3e11fe`にある`ggml-model-Q5_K_M.gguf`だけとする。モデルの差し替えや複数モデルへの対応を要件にしない。context 512 token、batch 512、microbatch 64、flash attention有効を原作準拠の推論プロファイルとし、thread数はNixOS上で利用可能なprocessor数に合わせる。llama.cppのsource revisionは、採用時点の`flake.lock`から一意に決まるものとする。

### GUIなしでの運用

設定GUIを提供しない。本プロジェクトの設定はNixOS設定から与える。候補表示にはFcitx5の標準機能を使用し、独自の候補ウィンドウを要件にしない。

### 入力操作と表示

日本語入力は、未入力、入力中、先頭候補preview中、候補選択中を区別する。ライブ変換は既定で有効にし、入力中は最上位の変換結果をプリエディットへ表示して候補一覧を隠す。Backspace直後は読みを表示する。ライブ変換を無効にした場合は入力中に先頭候補だけを標準候補UIへ提示し、最初のSpaceで先頭候補をpreview、次のSpaceで全候補選択へ移る。

候補選択は先頭候補から開始し、SpaceとDownで次、Shift+SpaceとUpで前へ進む。Escapeは入力を破棄せず、ライブ変換時は入力中へ、非ライブ時は先頭候補previewへ戻す。候補選択中のRightとEnterは選択範囲を確定し、残入力があれば再変換する。1から9は表示中の9件に対応し、0は現在のmarked textを確定して新しい入力として扱う。Tabは入力中に予測を受理し、予測がない場合も入力状態を保ったままconsumedとする。

通常候補と入力中予測は別の表示契約にする。候補はFcitx5標準候補UIへ最大9件単位で表示し、注釈をcommentへ渡す。予測は候補へ混ぜず、標準InputPanelの補助表示へ出す。入力中予測は既定で無効とし、有効時も現在の読みを厳密に延長する候補だけを表示する。確定後予測はconverter機能として保持するが、Desktop準拠の通常入力経路では自動表示しない。

### ソースとデータの固定

本システム、Fcitx5連携部分、llama.cppは、固定したソースからNixでビルドできなければならない。辞書は固定submoduleから取得する。GGUFモデルはflake内の`pkgs.fetchurl`でcommit `c67e03e07d215c869f591b274c1631170d3e11fe`のURLとhash `sha256-KcIj1MIzJ7gP0T67WrJVUFekYxeZfV2jkVhP++8NtnM=`を指定してrealizeし、NixOS moduleがそのNix store pathをdaemonへ渡す。daemonはモデルをダウンロードしない。

信頼するNixバイナリキャッシュから、同じ派生物の代替物を取得することは許容する。ソースとビルド定義を再現できない上流配布バイナリの再包装とは区別する。

### ライセンス境界

本プロジェクトが直接packageするデフォルト辞書、絵文字、tokenizerおよびGGUFモデルを別の資産として扱い、それぞれの出所、固定版、ライセンス、attributionおよび変更有無を記録しなければならない。nixpkgsからshared libraryとして利用する`pkgs.llama-cpp`は通常のpackage依存であり、この直接配布資産の台帳には含めない。

固定デフォルト辞書はApache-2.0であり、固定GGUFモデルは固定commitのmodel card metadataがApache-2.0を宣言している。原作内のtokenizer dataはCC BY-SA 4.0である。原作の生成済み絵文字辞書は、生成素材であるMozc dataのBSD-3-Clause、Unicode/CLDRのUnicode LicenseおよびazooKey独自追加分のMITを引き継ぐ。これはattributionだけの関係であり、Mozcのsource、library、daemonまたはpackageを製品依存にしない。各Nix packageには、該当するlicense本文とattributionを資産本体と一緒に格納する。固定辞書と固定モデルのrepositoryにはNOTICEファイルがないため、引き継ぐ上流NOTICEはない。

### NixOSのファイルシステム規約

動作のために`/usr`以下へファイル、ディレクトリ、シンボリックリンクを作成してはならない。Nixで管理する不変のプログラム、データおよびGGUFモデルはNixストアから参照する。

### 障害分離

変換処理またはモデル推論の異常が、Fcitx5プロセスを直接異常終了させない構成にする。

Fcitx5連携はfcitx5-mozcと同様に、daemon応答がconsumedとしたキーだけをacceptし、プリエディット、候補および確定結果をFcitx5標準APIへ反映する。daemonとの通信に失敗したキーはbufferまたは再送せず、セッションとUIをresetして未処理としてFcitx5へ返す。

## 現在の正常動作検証

2026-08-15に、NixOS 26.11、Linux 6.18.41、`x86_64-linux`、AMD Ryzen 5 4500U、15 GiB RAM、Fcitx5 5.1.21の環境で受け入れ確認を行った。ホストの入力セッションから分離したXvfb上で、Nix packageのFcitx5アドオン、daemon、固定辞書、固定GGUFモデルおよびGTK 4版Zenityを使用した。

### 機能シナリオ

| 機能群 | 外部から確認した正常動作 | 主な検証境界 |
| --- | --- | --- |
| 入力状態と入力方式 | 直接入力、ローマ字、AZIK、かなJIS/US、カスタム表、挿入、削除、カーソル移動 | converterの単体テストと`conversion_session_assets` |
| 辞書と基本変換 | 固定binary/LOUDS辞書、ラティス、全文・文節・単語変換、候補選択、確定 | `dictionary_*_assets`、`normal_conversion_assets`、`conversion_result_assets` |
| 予測と補完 | 入力中・確定後予測、英語・ギリシャ語、絵文字 | `input_prediction_assets`、`post_composition_prediction_assets`、`foreign_prediction_assets`、`emoji_assets` |
| 学習と拡張候補 | 学習、忘却、reset、学習mode、ユーザ辞書、特殊・表記・template候補 | `learning_memory_assets`、`user_dictionary_assets`、`typography_assets`、daemonの`engine_assets` |
| セッション操作 | live変換、部分確定、残入力の再変換、複数sessionの分離と復旧 | `conversion_session_assets`、daemonの`engine_assets`と`server_assets` |
| Zenzai | 通常・rich・personalization・次入力予測、LM誤入力訂正、model prefixによる制約付き再探索 | `zenz_*_assets`、daemonの`zenz_model_assets` |
| Fcitx5 | addonからのdaemon直接起動、プリエディット、候補1の選択、`司会`の確定 | 隔離した実Fcitx5・GTK 4アプリケーション |
| 障害復旧 | 入力途中でdaemonを`SIGKILL`してもFcitx5とアプリケーションが継続し、次のキー`x`を未処理のままアプリケーションへ返した | 隔離した実Fcitx5・GTK 4アプリケーション |

固定モデルを使うNix package testでは、flakeが固定する`ggml-model-Q5_K_M.gguf`と`pkgs.llama-cpp` 10273を実際にロードし、context 512、batch 512、microbatch 64、flash attention有効で辞書draft、model評価、prefix制約付き再探索から変換結果を得た。server executableも同じNixOS資産で要求を処理した。

### 性能と資源

- 入力contextを有効化してから固定モデルを読み込み、最初の要求を受けられるまで435 msだった。
- 有効化直後のdaemon RSSは180,896 KiBだった。変換2回後は186,148 KiB、4回後は186,176 KiB、6回後は186,188 KiB、8回後と10回後は186,200 KiBで、warm時の継続的な増加はなかった。
- 10回の変換で記録した81個のconsumed key pressは、最小2 ms、平均34.1 ms、p95 87 ms、最大90 msだった。
- addonの内部deadlineは5秒であり、計測したcold開始とwarm要求の双方に余裕がある。timeoutまたは切断時はsessionとUIをresetし、そのキーを未処理として返す。

## 完了条件

次をすべて満たしたときだけ、本プロジェクトを完成と判定する。

1. 本文書と`architecture.md`の未決事項のうち、実装対象に必要な項目が解決されている。
2. 固定したソースからNixOS向けパッケージをビルドでき、ビルドおよび実行時クロージャにSwift依存がない。
3. `programs.beankey.enable = true`によりFcitx5連携、変換デーモン、固定辞書および固定モデルを導入できる。
4. 固定原作で確認できるかな漢字変換機能をすべて実装し、各機能の代表シナリオで入力、候補、選択、確定、永続化および状態復旧が正常に動作する。
5. Zenzaiについて、辞書draft、モデル評価、prefix制約付き再探索のループが固定モデルで正常に変換結果を返す。
6. 確定した実環境でFcitx5から入力、候補選択、確定を行える。
7. 変換処理または推論を異常終了させても、Fcitx5と対象アプリケーションが異常終了せず、確定した入力保護動作を満たす。
8. 原作準拠の推論プロファイルで、cold/warm時の応答時間、メモリおよびモデル読込時間を記録し、実際の入力操作を阻害する停止または無制限な資源増加がない。
9. 本プロジェクトが直接packageするコードとデータについて、ライセンスおよび再配布条件を満たす。

上記1から9は、本文書の機能シナリオ、性能・資源記録、Nix build/check、closure監査および各資産packageに同梱したlicense・attributionによって満たした。固定原作との候補列、内部traceおよびlogitの厳密比較は、定義どおり後続の適合検証であり、現在の完成判定には含めない。

## 未決事項

現在の実装と完成判定を妨げる未決事項はない。Wayland、他のsystem architectureおよび固定原作との厳密な出力適合は、現在の受け入れ範囲を拡張するときに別途検証する。

## 後続作業

- 固定原作を実行するプラットフォームとFloat条件を定め、候補列、内部traceおよびlogitのgolden vectorを生成する。
- 同score候補、logit近接値およびbackend差の許容規則を定め、厳密な原作適合検証を行う。
- `pkgs.llama-cpp`と原作のllama.cpp `b4846`との挙動差を、将来の原作適合検証で記録する。

## 要件変更の扱い

- 対象範囲、対象外、現在の検証基準、将来の原作適合検証、要件または完了条件を変更する場合は、この文書を更新する。
- 実現方法だけを変更する場合は、`architecture.md`を更新する。
- 未決事項を解決した場合は、根拠と決定内容を本文へ反映する。
- 会話上の合意だけを、実装時の暗黙仕様として扱わない。

## 参考資料

- [AzooKeyKanaKanjiConverter固定コミット](https://github.com/azooKey/AzooKeyKanaKanjiConverter/tree/93766c46e31fa6a18b7ced49dab31337780f6f45)
- [Fcitx5](https://fcitx-im.org/wiki/Fcitx_5)
- [llama.cpp](https://github.com/ggml-org/llama.cpp)
