# AzooKeyKanaKanjiConverter 原作調査

- 調査対象: [azooKey/AzooKeyKanaKanjiConverter](https://github.com/azooKey/AzooKeyKanaKanjiConverter)
- 対象コミット: [`93766c46e31fa6a18b7ced49dab31337780f6f45`](https://github.com/azooKey/AzooKeyKanaKanjiConverter/commit/93766c46e31fa6a18b7ced49dab31337780f6f45)
- コミット日時: 2026-08-02 23:19:04 +09:00
- 調査日: 2026-08-14

## 調査範囲と読み方

この文書は、上記コミットの公式リポジトリ本体、同コミットが固定する公式サブモジュール、公式リポジトリの履歴・Release・Issue・Pull Requestを調べた結果である。他のLinux向け移植、第三者の移植記事、第三者実装、既存のローカル設計文書は参照していない。製品で固定するGGUFモデル`Miwa-Keita/zenz-v3.2-small-gguf`のcommit `c67e03e07d215c869f591b274c1631170d3e11fe`とnixpkgs packageについては、資産条件と原作比較に必要な範囲だけ公式配布元を追加確認し、その旨を明記する。

再実装を評価する場合も、変換精度そのものの良し悪し、他IMEとの比較、原作にない精度改善は対象にしない。対象に選んだ機能について、固定コミットと同じ条件・入力操作列・辞書・モデルを与えたときの入力状態遷移、候補生成と順位、辞書costの解釈、Zenzaiの入力と結果反映を再現できるかを評価軸とする。数値・backend差の境界は「再実装の同等性を評価する境界」で分けて述べる。

本文では、各項目を次の3区分に分ける。

- **確認済みの事実**: 固定コミットのコード、テスト、文書、または公式履歴から直接確認できたこと。
- **推論**: 複数の確認済み事実から合理的に導けるが、原作が仕様として明記してはいないこと。
- **未確認事項**: 公式リポジトリだけでは確定できないこと、または実資産・別プラットフォームでの実行確認が必要なこと。

`file:line` はrootの対象コミット、または同コミットが固定するsubmoduleの各HEADにおける行番号である。なお、対象は最新Release `v0.11.2` より後の `main` であり、リポジトリは1.0まで開発版としてマイナーバージョンでも破壊的変更を行う可能性を明記している（`README.md:35-36`）。

## 要約

### 確認済みの事実

AzooKeyKanaKanjiConverterは、入力履歴を持つ `ComposingText`、LOUDS辞書とViterbi系ラティス探索、候補整形を担う `KanaKanjiConverter` を中心とするかな漢字変換ライブラリである。通常変換に加え、ローマ字・AZIK・かな入力、入力中予測、確定後予測、学習、ユーザ辞書、特殊候補、複数入力セッション、Zenzaiによる候補評価、実験的LMベース誤入力訂正、CLIを持つ。

Zenzaiは、辞書候補の最終スコアを単純に置き換えるだけではない。辞書ラティスからdraft候補を作り、GGUFモデルで候補tokenを評価し、モデルが要求するUTF-8 prefixを制約として辞書ラティスへ戻し、制約付き探索と再評価を繰り返す。llama.cppはDarwinで `b4846` のXCFrameworkに固定され、Linux/Windowsでは同APIに対応するsystem libraryを要求する。

辞書・モデル・tokenizer・絵文字資産のライセンスは一様ではない。rootコードとデフォルト辞書はそれぞれMITとApache-2.0であり、本プロジェクトが固定するGGUFモデルは固定commitのmodel card metadataがApache-2.0を宣言している。tokenizer dataはCC BY-SA 4.0、絵文字資産は由来ごとの条件を持つ。製品のllama.cppはnixpkgsの通常依存として扱い、直接配布資産の監査対象にはしない。

### 推論

他言語再実装で最も高リスクなのは、単独の変換アルゴリズムより、入力履歴と表層文字列の対応、二重インデックスラティス、辞書バイナリ互換、候補の混合順、学習状態、Zenzaiのprefix制約ループを同時に整合させることである。

### 未確認事項

実際のIME UIにおけるライブ変換の呼出タイミング、全プラットフォームの性能条件および辞書の全生成元データのprovenanceは、今回の調査範囲だけでは確定できない。直接配布する辞書、GGUFモデル、tokenizerおよび絵文字については、固定revisionの明示条件とpackageへ添付すべきlicense・attributionを確認した。

## 1. 公開機能と主要ユースケース

### 確認済みの事実

公式READMEは本ライブラリをazooKeyのためのかな漢字変換エンジンと位置付け、`KanaKanjiConverter.withDefaultDictionary()`、`ComposingText`、`requestCandidates`による組込み例を示している（`README.md:3-5`, `README.md:39-70`）。

公開される主要機能は次のとおりである。

| 機能 | 外部境界・用途 | 主な根拠 |
| --- | --- | --- |
| 通常のかな漢字変換 | `requestCandidates`から全文・文節・単語候補を得る | `KanaKanjiConverter.swift:1190-1211` |
| 入力状態管理 | 直接入力、ローマ字、AZIK、かなJIS/US、カスタム表、挿入・削除・カーソル・文頭確定 | `ComposingText.swift:18-65`, `InputTable.swift:85-104`, `Docs/composing_text.md:50-87` |
| 入力中予測 | 日本語予測をmainへ混在、別配列へ分離、または無効化 | `ConvertRequestOptions.swift:12-24`, `ConversionResult.swift:1-10` |
| 確定後予測 | 品詞・末尾語・ゼロヒント・絵文字から次の候補を生成 | `KanaKanjiConverter.swift:1228-1285` |
| 英語・ギリシャ語予測 | OSのSpellCheckerを用いる補完 | `KanaKanjiConverter.swift:533-615` |
| 学習 | 候補選択の一時学習、永続化、忘却、リセット | `KanaKanjiConverter.swift:441-478` |
| ユーザ辞書 | 動的辞書・全文一致shortcut、ファイル辞書URL更新 | `KanaKanjiConverter.swift:416-429` |
| 特殊候補 | 暦、メール、Unicode、バージョン、時刻、カンマ区切り数値 | `KanaKanjiConverter.swift:51-58` |
| 表記候補 | ひらがな、カタカナ、大文字、全角英数、半角かな | `KanaKanjiConverter.swift:761-837` |
| テンプレート候補 | 日付・乱数タグを候補確定前に展開 | `Candidate.swift:186-221` |
| Zenzai | GGUFモデルを使った候補評価と制約付き再探索 | `zenzai.swift:123-160`, `ZenzCandidateEvaluator.swift:80-150` |
| 実験的LM誤入力訂正 | ZenzまたはN-gramを使ったbeam search候補 | `KanaKanjiConverter.swift:349-391` |
| 複数入力セッション | 1つのConverterで独立したcomposition状態を切替 | `KanaKanjiConverter.swift:16-35`, `KanaKanjiConverter.swift:86-125` |
| CLI | 単発変換、対話セッション、評価、辞書build/read、N-gram train/inference | `Docs/cli.md:39-156` |

`ConversionResult` は候補を `mainResults`、日本語の `predictionResults`、英語の `englishPredictionResults`、`firstClauseResults` に分けて返す（`ConversionResult.swift:1-10`）。`Candidate` は表示文字列だけでなく、スコア、消費する入力数、辞書要素列、確定時のカーソルaction、学習対象フラグを持つ（`Candidate.swift:143-178`）。`KanaKanjiConverter` はスレッドセーフではなく、複数の実行contextから使う場合は呼出側の直列化を要求する（`KanaKanjiConverter.swift:108-112`）。

### 推論

中心ユースケースはIMEまたはキーボードアプリへの組込みである。ライブラリは候補UIそのものを持たず、入力側が `ComposingText` を更新し、候補表示・選択・確定・学習APIを組み合わせる設計である。CLIは同じ公開境界を対話的に検証する参照クライアントとして利用できる。

### 未確認事項

- 実際のiOS/macOS/visionOSアプリ側のUI統合コードはこのリポジトリに含まれない。
- READMEはiOS 16以降を記載する一方、対象コミットのSwiftPM platform宣言はiOS 17、macOS 13である（`README.md:7-8`, `Package.swift:176-179`）。各platformの実際の下限は原作側でも記述が一致していない。
- 他言語向けの安定ABIやFFIは定義されていない。

## 2. パッケージ、モジュール、主要型の構造と責任

### 確認済みの事実

SwiftPM packageは次の主要targetから成る（`Package.swift:34-105`, `Package.swift:163-174`）。

| Target / product | 責任 |
| --- | --- |
| `SwiftUtils` | 文字種変換、Collection/String/Data補助 |
| `EfficientNGram` | Zenz tokenizer、Kneser-Ney N-gramの学習・推論 |
| `KanaKanjiConverterModule` | 入力管理、辞書、ラティス変換、候補、学習、Zenzai本体。辞書資産は含まない |
| `KanaKanjiConverterModuleWithDefaultDictionary` | デフォルト辞書と絵文字辞書をresourceとして束ね、factory APIを追加 |
| `CliTool` | `anco` CLI |
| 4 test targets | 上記target別のXCTest |
| `llama.cpp` | Darwinではbinary target、Linux/Windowsではsystem library |

主要外部packageはSwift Algorithms、Swift Collections、Swift Argument Parser、swift-tokenizersである。SwiftyMarisaは対象platformとtraitに応じて追加される（`Package.swift:14-31`）。Zenzai機能はSwiftPM trait `Zenzai` または `ZenzaiCPU` で有効化される（`Package.swift:196-200`）。

主要型の責任は次のとおりである。

| 型 | 責任 | 根拠 |
| --- | --- | --- |
| `ComposingText` | 入力要素列、表層かな列、表層カーソルの整合を保ちながら編集する | `ComposingText.swift:18-65`, `ComposingText.swift:145-374` |
| `InputPiece` / `InputStyle` | 文字・キーと、直接入力・roman2kana・mapped tableを表現する | `InputPiece.swift:1-9`, `InputStyle.swift:1-11` |
| `InputTable` / `InputStyleManager` | 入力規則のtrie、組込み表、custom tableのload/register/exportを担う | `InputTable.swift:85-151`, `InputStyleManager.swift:5-31` |
| `KanaKanjiConverter` | 公開API、辞書状態、変換session、候補要求、学習、Zenz modelを統括する | `KanaKanjiConverter.swift:14-74` |
| `Kana2Kanji` | 辞書検索、ラティス構築、Viterbi/N-best、予測、Zenzai制約探索を担う | `Kana2Kanji.swift:17-50`, `FullInputProcessing.swift:13-101` |
| `Lattice` | input indexとsurface indexの二重indexでnodeを保持する | `Lattice.swift:15-110` |
| `LatticeNode` / `RegisteredNode` | 辞書nodeと、それに到達するN-best pathを保持・復元する | `LatticeNode.swift:11-65`, `RegisteredNode.swift:121-190` |
| `DicdataStore` | LOUDS、語彙、CID/MID cost、予測、memory/user辞書を読み出す | `DicdataStore.swift:13-64`, `DicdataStore.swift:639-697`, `DicdataStore.swift:873-955` |
| `DicdataElement` | 表記、読み、左右接続ID、意味ID、基礎値・補正・metadataを表す | `DicdataElement.swift:11-64` |
| `Candidate` / `ConversionResult` | UIへ返す候補と候補群を表す | `Candidate.swift:143-178`, `ConversionResult.swift:1-10` |
| `LearningManager` | 一時・長期学習辞書、永続化、復旧を担う | `LearningMemory.swift:608-648`, `LearningMemory.swift:651-854` |
| `Zenz` / `ZenzContext` | model/contextの共有、推論の直列化、tokenize/decode/KV cacheを担う | `Zenz.swift:5-40`, `ZenzContext.swift:268-378` |
| `ZenzPromptBuilder` | model世代別promptと文脈tagを構築する | `ZenzPromptBuilder.swift:4-150` |
| `ZenzCandidateEvaluator` | draft candidateをtoken単位で評価し、通過または修正prefixを返す | `ZenzCandidateEvaluator.swift:80-150`, `ZenzCandidateEvaluator.swift:210-379` |
| `AncoSession` | CLIにおける入力、候補選択、文脈、replayの状態機械 | `AncoSession.swift:4-15`, `AncoSession.swift:159-396` |

`KanaKanjiConverter` 内では、ラティス、前回入力、確定候補、Zenzai incremental cacheなどを `ConversionSessionState` ごとに分離する。一方、辞書、モデル、学習データ、Converter単位のZenzai memoizationは共有する（`KanaKanjiConverter.swift:25-35`, `KanaKanjiConverter.swift:86-90`）。

### 推論

`KanaKanjiConverterModuleWithDefaultDictionary` は別アルゴリズムではなく、資産同梱とfactoryを担う配布境界である。再実装では、変換coreと辞書資産の配布を分離できる。

### 未確認事項

package内部型のどこまでを将来も維持するかは明記されていない。対象は1.0未満であるため、Swiftの現在の公開型集合をそのまま他言語の永久的API契約と見なすことはできない。

## 3. 入力状態、かな漢字変換、候補生成・選択・確定の処理経路

### 確認済みの事実

#### 3.1 入力状態

`ComposingText` は `input: [InputElement]`、`convertTarget: String`、`convertTargetCursorPosition: Int` を持つ（`ComposingText.swift:18-30`）。`input` は利用者が行った入力と入力styleを保存し、`convertTarget` はローマ字かな変換後に画面へ見せる表層文字列である。これにより、日本語ローマ字入力と直接英字入力のような見た目が同じ入力を区別する（`Docs/conversion_algorithms.md:13-54`）。

中間カーソルへの挿入・削除では、表層位置から入力位置を逆算し、必要に応じて既存roman segmentを直接かなへ凍結してから編集する（`ComposingText.swift:145-199`, `ComposingText.swift:206-327`）。したがって、`input.count` と `convertTarget.count` は一般に一致しない。

#### 3.2 変換要求

`requestCandidates` の処理順は次のとおりである（`KanaKanjiConverter.swift:1190-1211`）。

1. 表層入力が空なら空結果を返す。
2. deprecated optionが指定された場合は学習をresetする。
3. request optionに応じて辞書・学習状態を更新する。
4. classic typo correctionの有効性を決める。`.automatic` はiOSのみ有効で、他OSでは無効である（`KanaKanjiConverter.swift:1213-1225`）。
5. `convertToLattice` で通常またはZenzai経路のラティスを得る。
6. `processResult` でUI向け候補群へ変換する。

#### 3.3 通常ラティス経路

`convertToLattice` は状態により次を選ぶ（`KanaKanjiConverter.swift:1093-1172`）。

- 初回: `kana2lattice_all`
- 同一入力: `kana2lattice_no_change`
- 文節確定後: `kana2lattice_afterComplete`
- 末尾追加・削除・置換: `differenceSuffix`を使う `kana2lattice_changed`

初回処理はinput/surfaceの対応mapを作り、各開始位置から辞書を検索してnodeを追加し、単語cost・接続costを加えながら各nodeへN-best pathを登録する（`FullInputProcessing.swift:30-101`）。原作はこの基盤をViterbiと説明し、文節化後にMIDによる内容語共起costを追加する（`Docs/conversion_algorithms.md:5-11`）。

二重indexラティスは、たとえばroman列 `ittai` に対して表層「イッ」に対応する入力substringが存在しない問題を扱う。通常語彙探索はsurface index、typo correctionはinput indexも使う（`Docs/conversion_algorithms.md:123-131`）。

#### 3.4 候補構成と順位

`processResult` はラティスpathを `CandidateData` に復元し、全文候補、予測、英語補完、shortcut、文節候補、辞書単語、表記候補、特殊候補を重複排除しながら組み合わせる（`KanaKanjiConverter.swift:850-1084`）。重要な規則は次のとおりである。

- 全文候補のうち上位5件をmain先頭へ使う。
- 日本語予測を有効化した場合は最大3件を作り、`.autoMix` のときだけmainへ混ぜる（`KanaKanjiConverter.swift:921-949`）。
- 英語予測もoptionに応じて分離または混合する（`KanaKanjiConverter.swift:950-962`）。
- main先頭は最大5件にし、上位3件以内にtypoではない「読みが入力へ完全一致する候補」を最低1件入れる（`KanaKanjiConverter.swift:1050-1063`）。
- その後に文節候補、先頭辞書nodeと表記候補を追加し、おおむね読みの長さ優先・同長ならcost優先とする（`KanaKanjiConverter.swift:983-1046`）。
- 最後にカーソルactionと日付・乱数templateを適用する（`KanaKanjiConverter.swift:1065-1084`）。

Zenzai有効時は、Zenzaiが返した順序を後段の既存rerankで保つため、先頭5件の `value` をその順へ付け替える暫定実装がある（`KanaKanjiConverter.swift:905-919`）。

#### 3.5 候補選択と確定

CLIの参照経路では、選択candidateを `setCompletedData` と `updateLearningData` へ渡し、candidateの `composingCount` だけ `ComposingText.prefixComplete` し、残入力があれば再変換する。空になれば `stopComposition` する（`AncoSession.swift:301-323`）。学習のmemory更新と永続化は分離され、後者には `commitUpdateLearningData` を呼ぶ（`KanaKanjiConverter.swift:441-468`）。

### 推論

再実装の観測可能な最小境界は、入力操作列、各時点の `convertTarget` とcursor、候補文字列・順序・消費範囲、確定後の残入力、学習後の再順位である。top-1文字列だけの比較では、入力状態や部分確定の互換性を検証できない。

### 未確認事項

- UI側がいつ `setCompletedData`、`updateLearningData`、`commitUpdateLearningData` を呼ぶかは製品実装に委ねられる。
- 辞書costとcandidate混合規則はコードで確認できるが、全入力に対する安定した外部仕様表は存在しない。

## 4. ライブ変換など通常変換以外の機能と依存関係

### 確認済みの事実

#### ライブ変換

公式文書上のライブ変換は専用探索ではない。通常どおり候補を要求し、「予測ではない完全一致候補のうち最上位」を表示する（`Docs/conversion_algorithms.md:191-193`）。同文書はライブ変換中の中間カーソル移動を扱わない方針を述べる（`Docs/conversion_algorithms.md:72-111`）。Zenzaiの `requestRichCandidates` も処理量が増えるため、通常はライブ変換に非推奨である（`ConvertRequestOptions.swift:241-248`）。

#### 予測変換

入力中予測は末尾の1〜数文節を前方一致検索し、候補を作る（`KanaKanjiConverter.swift:617-682`, `Prediction.swift:145-251`）。roman入力では未確定suffixから可能な次のかなを列挙する（`Prediction.swift:101-143`）。通常辞書予測が空の場合に限り、experimental flagが有効ならZenz v3で次入力を生成し、それをscratch sessionで通常変換するfallbackがある（`KanaKanjiConverter.swift:683-727`）。

確定後予測は、品詞に基づくzero-hint、末尾単語から長い複合語を引くreplacement、絵文字を混ぜる（`Docs/conversion_algorithms.md:195-226`, `KanaKanjiConverter.swift:1228-1285`）。

#### 学習・ユーザ辞書

学習は一時memoryと長期memoryを用い、選択candidate中の単語、文節、全文を新たな辞書entryとして優先する。使用回数は32日ごとに半減する（`Docs/conversion_algorithms.md:133-157`）。長期保存は `.2` 一時ファイルと `.pause` markerを使い、中断後の復旧を行う（`Docs/conversion_algorithms.md:159-179`）。

#### 特殊候補・外部依存

デフォルトのspecial providerはCalendar、EmailAddress、Unicode、Version、TimeExpression、CommaSeparatedNumberである（`KanaKanjiConverter.swift:51-58`）。追加providerとしてTypographyも型レベルでは提供されるが、対象コミットのdefault配列には入っていない（`SpecialCandidateProvider.swift:46-78`）。英語・ギリシャ語補完はFoundationのspell checker利用可否に依存し、非Apple分岐は常に`nil`を返す（`SpellChecker.swift:8-13`, `SpellChecker.swift:39-80`, `KanaKanjiConverter.swift:399-413`）。絵文字置換はdefault emoji resourceへ依存する。

NixOSでこの公開機能を正常動作させるため、固定nixpkgs `0e251e24a4f24e036a084b6b4b2d2491af4167f4`のHunspell 1.7.3 C APIと`hunspellDicts.en_US`、`hunspellDicts.el_GR`を利用する。Hunspellはspell checkとsuggestionを提供するがAppleのprefix completion APIではないため、suggestionのうち入力を大文字小文字無視で前方一致するものだけを補完として扱う。固定packageによる実測では、`hel`から`heel`、`hell`、`held`、`helm`、`help`を、`καλ`から`καλά`、`καλό`、`καλέ`などを取得できる。el_GR辞書は`ISO8859-7`を宣言するため、C API入出力をUTF-8との間で変換する。

#### LMベース誤入力訂正

通常変換のclassic typo correctionとは別に、`experimentalRequestTypoCorrection` がZenzまたはEfficientNGramを使う候補を返す。このAPIは予告なく変更・削除し得ると明記される（`KanaKanjiConverter.swift:349-391`）。N-gram経路はtokenizerと4個のMarisa trie fileを要求する（`EfficientNGram/Inference.swift:30-45`）。

### 推論

ライブ変換は通常変換の結果選択policyとして実装できる一方、滑らかな逐次体験には差分ラティス、prediction安定化cache、roman未確定suffix、Zenzai cacheの性能特性が関わる。そのため、機能上のライブ変換と、実用品質のlatencyは別の完了条件である。

### 未確認事項

- 製品UIが完全一致候補をどのタイミングで表示・自動確定するかは、このengineリポジトリだけでは決まらない。
- Apple spell checkerとHunspellの候補列、語彙および順位の同等性はなく、後続の原作適合検証でもplatform差として分離する。

## 5. 辞書形式、生成・読み込み、必要なデータ資産

### 確認済みの事実

#### 資産構成

READMEが示す辞書rootは `louds/`、`p/`、`cb/`、`mm.binary` から成る（`README.md:136-157`）。対象コミットのdefault dictionary submoduleは `4d418525b090cf49c219819d05a7e3cc2a4346eb` で、実際にはおおよそ次を含む。

- `louds/charID.chid`
- 先頭文字ごとの `[UTF16]...louds`、`.loudschars2`、複数の `.loudstxt3` shard
- `p/pc_<CID>.csv`
- `cb/<CID>.binary`
- `mm.binary`

`DicdataElement` は `word`、カタカナ `ruby`、`lcid`、`rcid`、`mid`、基礎値、動的 `adjust`、metadataを持つ（`Docs/dicdata_format.md:7-34`, `DicdataElement.swift:11-64`）。

#### LOUDSとentry payload

公式文書は、`.louds` をLOUDS trieのbit列、`.loudschars2` を各nodeの1-byte character ID、`charID.chid` をCharacter-ID対応、`.loudstxt3` をnodeのentry dataと説明する（`Docs/dicdata_format.md:36-56`）。

読み込みは次の順である。

1. 起動時に `charID.chid` をSwift `Character` 列として読み、列挙offsetをbyte IDとする（`DicdataStore.swift:40-49`）。
2. queryをID列へ変換し、先頭文字identifierの `.louds` と `.loudschars2` から `LOUDS` を作る（`extension LOUDS.swift:39-57`）。
3. trieでnode indexを得る。
4. 対応 `.loudstxt3` shardをmapped readし、`DicdataElement` へ復元する（`extension LOUDS.swift:204-229`）。

`DictionaryBuilder` は1 shard当たり `1 << 11 = 2048` slotを使う（`DictionaryBuilder.swift:4-11`）。通常identifierは先頭のSwift `Character` を構成するUTF-16 code unitを4桁大文字hexで囲んだ名前であり、`user`、`memory`、`user_shortcuts` は予約名としてそのまま使う（`DictionaryBuilder.swift:156-182`）。これはUnicode scalar単位ではない。

`.loudstxt3` の各entry blockは、`UInt16` entry数、各entryの `UInt16 lcid/rcid/mid` と `Float32 score`、その後の `ruby` と `word` のUTF-8/TAB区切りpayloadから成る。`word == ruby` のときwordは空欄へ省略される（`DictionaryBuilder.swift:273-375`）。readerはlittle-endian数値として復元し、空wordをrubyへ戻す。一方、対象builderは整数・Floatのnative memory bytesを書いている（`extension LOUDS.swift:13-37`, `extension LOUDS.swift:99-160`, `DictionaryBuilder.swift:299-330`）。

#### 接続・意味costと予測資産

- `cb/<former>.binary` はsparseな `(Int32, Float)` pairを格納し、1319要素のconnection costへ展開する。key `-1` はdefault値で、file不在時のdefaultは `-25` である（`DicdataStore.swift:873-924`）。
- `mm.binary` は `midCount = 502` のdense matrixで、MID 500のBOS/EOSは常に0として扱う（`DicdataStore.swift:19-35`, `DicdataStore.swift:943-955`）。
- `p/pc_<lastRcid>.csv` は確定後の品詞ベース予測に使う（`DicdataStore.swift:639-657`）。

#### 生成

public `DictionaryBuilder` は `DicdataElement` 配列と `charID.chid` mappingからLOUDS・character array・entry shardを書き出せる（`DictionaryBuilder.swift:11-83`）。CLI `anco dict build` は `worddict/<target>.tsv` の6列 `ruby, word, lcid, rcid, mid, score` を読み、LOUDS、character ID、CID cost、MID matrixを生成する（`BuildCommand.swift:13-20`, `BuildCommand.swift:28-73`, `BuildCommand.swift:104-220`）。

#### 文書とのdrift

対象コミットでは、READMEの例が `charId.chid`、format文書が `.charID` と書く一方、実コードと資産は `charID.chid` を要求する（`README.md:143-156`, `Docs/dicdata_format.md:36-49`, `DicdataStore.swift:45-49`）。また、変換algorithm文書は旧 `p/p_あ.csv` を説明するが、対象default辞書の実資産はUnicode escape済みLOUDS名と `p/pc_*.csv` を持つ。辞書formatは文書だけでなく、対象コミットのbuilder/reader/fixtureを正とする必要がある。

### 推論

他言語実装では、辞書data model、LOUDS traversal、node index、2048単位shard、UTF-16 identifier、UTF-8 payloadを別々に検証する必要がある。特にSwift `Character`、UTF-16、UTF-8を一種類の「文字」として扱う実装は互換にならない。

### 未確認事項

- `Docs/dicdata_format.md` は `.loudschars2`、character ID、CID/MID binary詳細を `TBW` としており、独立した正式format specificationは完成していない（`Docs/dicdata_format.md:58-82`）。
- `.loudstxt3` のreaderはlittle-endianを明示するが、builderはnative memory bytesを書き、`mm.binary` と `cb/*.binary` にもnative表現を介する箇所がある。現在のlittle-endian環境以外を含むportableなendianness/alignment契約は確認できない。
- malformed/truncated辞書に対する必須error behaviorは定義されていない。
- default dictionaryの元corpus全体のprovenanceは、固定submoduleのREADMEが `TBW` のため確認できない。

## 6. Zenzaiの処理経路、モデル入力、周辺文脈、候補への反映

### 確認済みの事実

#### 有効化とmodel世代

ZenzaiはSwiftPM trait `Zenzai`（GPU offload可）または `ZenzaiCPU`（GPU offload無効）と、`ConvertRequestOptions.zenzaiMode = .on(...)` で有効化する。入力はGGUF weight URL、推論回数上限、rich candidate flag、任意のpersonalization N-gram、model世代別configである（`README.md:106-133`, `ConvertRequestOptions.swift:217-263`）。

公式文書におけるmodel prompt formatは次のとおりである（`Docs/zenzai.md:39-44`）。

ここでいうv1、v2、v3はZenzaiのmodelおよびprompt世代名であり、本プロジェクトの製品段階ではない。

| 世代 | 基本format・追加文脈 |
| --- | --- |
| v1 | `EE00 + input_katakana + EE01 + output + EOS` |
| v2 | v1に `EE02 + left context` を追加。入力後にcontextを置く形式も持つ |
| v3 | conditions、left contextを入力前へ置く。`EE03` profile、`EE04` topic、`EE05` style、`EE06` preference |
| v3.2 | v3に `EE07` right contextを追加 |

対象実装はさらに `EE08` alignment separatorを持ち、中間cursor位置を入力promptと候補末尾へ対応付ける（`ZenzPromptBuilder.swift:4-9`, `ZenzPromptBuilder.swift:130-150`, `ZenzCandidateEvaluator.swift:382-397`）。

#### 周辺文脈の構築

v3 conditionのprofile/topic/style/preferenceは各末尾25文字へ切る。left contextは末尾、right contextは先頭を使う。文脈がなければ空文字列になり、文脈があって最大長指定がなければ最大40文字を使う（`ZenzPromptBuilder.swift:11-50`）。候補中にuser dictionary entryがあると、v2では `辞書:表記(よみ)` condition、v3でもcondition文字列としてpromptへ入る（`ZenzCandidateEvaluator.swift:92-102`, `ZenzPromptBuilder.swift:83-134`）。spaceは全角spaceへ、新lineは削除してtokenizeする（`ZenzContext.swift:586-605`）。

#### 辞書draftとmodel評価のloop

`convertToLattice` はZenzai有効時に `all_zenzai` へ分岐する（`KanaKanjiConverter.swift:1098-1115`）。処理は次のloopである。

1. 中間cursorならcursorより前だけを辞書ラティス対象とし、model promptにはcursor位置を渡す（`zenzai.swift:478-484`）。
2. 通常の `kana2lattice_all`、または既存prefix constraintを使う `kana2lattice_all_with_prefix_constraint` でdraft候補を作る（`zenzai.swift:255-312`）。
3. 辞書cost上のbest candidateを `ZenzCandidateEvaluator` へ渡す（`zenzai.swift:313-368`）。
4. evaluatorはpromptとcandidateをtokenizeし、各candidate tokenが直前位置の最大logit tokenか順に調べる（`ZenzCandidateEvaluator.swift:146-169`, `ZenzCandidateEvaluator.swift:210-327`）。
5. 全tokenが一致すれば `.pass`。別tokenが最大なら、そのtokenまでをUTF-8 prefix constraintとして `.fixRequired`。modelがEOSを選べば、そこまでの全文を `.wholeResult` とする（`ZenzCandidateEvaluator.swift:300-379`）。
6. prefixを満たす既存candidateがあれば再評価し、なければそのprefixで辞書ラティスを再探索する。`inferenceLimit`まで繰り返す（`zenzai.swift:329-468`, `zenzai.swift:486-580`）。
7. rich modeでは最大logit以外の上位tokenも確率比付きalternative constraintとして保持し、比率に応じて追加candidateを探す（`ZenzCandidateEvaluator.swift:210-220`, `ZenzCandidateEvaluator.swift:328-379`, `zenzai.swift:376-416`）。

つまりZenzaiの出力語彙は辞書candidateを選ぶ制約として使われ、原則として辞書ラティスから最終candidateを構成する。user/learned candidateがrejectされたときは、その影響を無視した再探索へ切り替える処理もある（`zenzai.swift:501-540`, `zenzai.swift:541-580`）。

#### personalizationと学習candidate

personalization modeはbase/personal N-gramのtoken probability差を `alpha` でmodel logitへ加える（`ZenzCandidateEvaluator.swift:223-269`）。learned entryを含むcandidateには読み長に応じる優先logitを加え、model最大tokenとの差を覆せる（`ZenzCandidateEvaluator.swift:170-188`, `ZenzCandidateEvaluator.swift:300-327`, `ZenzCandidateEvaluator.swift:400-409`）。

#### 次入力予測とLM typo correction

Zenz v3の次入力予測は、conditions + left/right context + `EE00` + katakana composing textをpromptとし、反復penalty、entropy threshold、句読点stop、roman tableから得た許容prefixを使ってgreedyに文字を生成する（`ZenzPromptBuilder.swift:53-73`, `ZenzInputTextGenerator.swift:9-125`）。

実験的typo correctionは `ZenzCompatibleInputLanguageModelContext` を介し、ZenzとEfficientNGramに共通のtokenize・語彙・next-log-prob interfaceを使う（`ZenzCompatibleInputLanguageModelContext.swift:8-16`, `KanaKanjiConverter.swift:349-391`）。

#### cacheと並行性

同じmodel URLのread-only weightとnative contextはprocess内で共有し、推論はlockで直列化する（`Zenz.swift:5-40`, `Zenz.swift:67-95`）。evaluation、draft、resolved conversion、prompt tokensのmemoizationはConverter単位であり、`stopComposition` 後も保持でき、明示的purgeが可能である（`ZenzContext.swift:197-265`, `KanaKanjiConverter.swift:142-155`）。

### 推論

Zenzai互換を「最終candidateのreranker」とだけ定義すると不十分である。互換性にはprompt bytes、tokenization、最大logit比較、UTF-8 prefix、辞書制約探索、inference limit、user/learning例外、cache stateまで関係する。

### 未確認事項

- 実GGUFなしでmodelのlogit・token列そのものは確認できない。
- v1用の現行public configはなく、現在のcode pathがv1をどこまで実用互換として扱うかは確認できない。
- model学習時のcorpus、loss、quantization手順はこのリポジトリに含まれない。

## 7. llama.cppへ依存する機能・API・バージョン制約

### 確認済みの事実

#### 配布境界と固定版

Darwinでは、AzooKey forkのRelease `b4846` にあるsigned XCFrameworkをchecksum付きbinary targetとして取得する。Linux/Windowsではmodule mapが `llama` libraryへlinkするsystem library targetである（`Package.swift:142-170`, `Sources/llama.cpp/module.modulemap:1-10`）。llama.cpp依存は `Zenzai` / `ZenzaiCPU` traitのときだけconverter targetへ接続される（`Package.swift:163-173`）。traitなしでは `llama-mock.swift` の型・関数stubを用いる（`llama-mock.swift:1-67`）。

対象コミットは `b9637-azookey.1` ではなく `b4846` を意図的に固定する。公式コメントと[PR #351](https://github.com/azooKey/AzooKeyKanaKanjiConverter/pull/351)によれば、b9637はmacOS Apple Silicon CPUで約10.8から7.2 ms/requestへ高速化した一方、約63.77 MiBのrepack bufferを追加し、iOS逐次direct入力の回帰とMetal終了時assertが残った。iOS実機でb4846へ戻すと回復したため、small-batch性能、repack memory、lifecycleを再評価できるまで固定する（`Package.swift:145-159`）。

#### 実際に使うAPI群

| 目的 | API |
| --- | --- |
| backend/model | `llama_backend_init`, `llama_model_default_params`, `llama_model_load_from_file`, `llama_model_get_vocab`, `llama_model_free` |
| CPU-only device | `ggml_backend_dev_by_type`, `GGML_BACKEND_DEVICE_TYPE_CPU`, `LLAMA_SPLIT_MODE_NONE` |
| context | `llama_context_default_params`, `llama_init_from_model`, `llama_free`, `llama_n_ctx` |
| batch/decode | `llama_batch_init/free`, batch field設定, `llama_decode`, `llama_get_logits` |
| token | `llama_tokenize`, `llama_vocab_bos/eos/n_tokens`, `llama_token_to_piece` |
| incremental KV | `llama_kv_cache_seq_rm`, `llama_kv_cache_seq_cp`, `llama_kv_cache_seq_pos_max` |

実装箇所は主に `ZenzContext.swift:268-378`, `ZenzContext.swift:433-518`, `ZenzContext.swift:545-629` である。

model loadはmmapを有効にする。`ZenzaiCPU` ではGPU layerを0、split modeをnone、device listをCPUだけにし、contextのKQV offloadも無効化する（`ZenzContext.swift:273-303`, `ZenzContext.swift:433-446`）。contextは512 token、batch 512、microbatch 64、flash attention有効である（`ZenzContext.swift:342-378`）。

固定GGUFは`tokenizer.ggml.pre = gpt2-small-japanese-char`を持つ。原作が固定するazooKey/llama.cpp `b4846`では、この値を専用enumへ対応付けるが、実際のpre-tokenization regexは`gpt-2`と同じである（`src/llama-vocab.cpp:341-348,1617-1618`）。固定nixpkgsのllama.cpp `10273`は専用名を受理しない一方、`gpt-2`を同じregexへ対応付けるため、製品はmodel load時の`llama_model_kv_override`で`tokenizer.ggml.pre`だけを`gpt-2`へ読み替える。GGUF本体は変更しない。

evaluationとinput predictionに別sequence IDを使い、完全token prefixがより長い場合はKVをsequence間copyし、分岐後をremoveしてdecodeする（`ZenzContext.swift:465-518`）。候補評価、次入力生成、greedy decode、Zenz-based typo correctionがこのlogit取得経路に依存する（`Zenz.swift:67-157`）。

### 推論

原作側の挙動を記録するには、上記C API semanticsとb4846のKV cache behaviorを含む実行条件を固定する必要がある。新しいllama.cppへ機械的に置換すると、compile互換だけでなく、small-batch latency、memory footprint、lifecycle、KV prefix再利用の結果を再検証する必要がある。本プロジェクトの製品runtimeは原作と同じrevisionを要求せず、`flake.lock`が固定するnixpkgsの`pkgs.llama-cpp`を別条件として固定し、原作オラクルとの差を測定する。

### 未確認事項

- b4846 Release archiveの署名手順と各platform binaryのbuild optionはこのrepoだけでは確認できない。本プロジェクトはb4846を製品で利用または再配布しないため、そのlicense bundleは製品配布の確認対象ではない。
- Linux/Windowsで利用するsystem libraryの正確なSONAME、配置、全backend plugin条件はSwiftPM manifestでは規定されない。公式CIはb4846 binaryを別途downloadしてlibraryを配置する（`.github/workflows/swift.yml:31-94`）。
- b4846より新しい版で回帰原因が解消済みかは、対象コミット時点で未確認である。

## 8. テスト構成と再実装時のオラクル

### 確認済みの事実

対象コミットには42個のSwift test fileと、名称が `test...` のXCTest methodが200個ある。targetは `SwiftUtilsTests`、`EfficientNGramTests`、`KanaKanjiConverterModuleTests`、`KanaKanjiConverterModuleWithDefaultDictionaryTests` である（`Package.swift:78-105`）。

#### オラクルとしての層

| 層 | 使えるtest / fixture | 固定できること |
| --- | --- | --- |
| 入力状態 | `ComposingTextTests`, `KeyInputTests`, `InputTablesTests`, `InputTableFormatTests` | insert/delete/cursor、roman/AZIK/kana/custom table、input-surface対応 |
| 辞書byte/LOUDS | `DictionaryMock/`, `DictionaryBuilderTests`, `LOUDSTests` | file生成・read round-trip、node検索、shard、identifier、entry field |
| ラティス | `LatticeTests`, `RegisteredNodeTests`, `ClauseDataUnitTests` | dual index、path登録、candidate復元 |
| 通常変換 | default dictionary `ConverterTests`, `ScenarioTests` | direct/roman/AZIK逐次入力、削除、予測安定性、must/must-not、top-1 |
| 特殊候補 | Calendar/Email/Unicode/Time/Comma/Template/JapaneseNumber tests | 入力と出力文字列の完全一致 |
| 学習 | `LearningMemoryTests`, `TemporalLearningMemoryTrieTests` | update、save、reset、forget、`.pause`復旧、生成file |
| session/確定 | `ConverterSessionTests`, `AncoSessionTests`, `AncoSessionRequestTests` | session分離、候補選択・確定・残入力 |
| Zenz純粋部分 | `ZenzPromptBuilderTests`, cache tests | prompt tag、文脈trim、right context、separator、cache policy |
| Zenz実model | `ZenzaiTests` | 実GGUFでのfull/incremental/予測/typo結果とlatency計測経路 |
| N-gram/tokenizer | `EfficientNGramTests` | 固定token ID、fast/slow一致、train/inference |

`DictionaryMock` には `charID.chid`、`シ.louds`、`シ.loudschars2`、`シ0`〜`シ13.loudstxt3`、`p_シ.csv`、`cb/1285.binary`、`mm.binary` があり、test resourceとしてpackageされる（`Package.swift:90-97`, `Tests/KanaKanjiConverterModuleTests/DictionaryMock/`）。`LOUDSTests` は「シカイ」から `司会`、`視界`、`死界` を得ることを固定する（`LOUDSTests.swift:12-47`）。

`DictionaryBuilderTests` はuser dictionaryと先頭文字別dictionaryのround-trip、binary writer/parser、word省略規則、空白・slash・ASCII・日本語・絵文字・予約語identifier、2048 slot、custom shard shiftを検証する（`DictionaryBuilderTests.swift:83-409`）。これは辞書互換の最も直接的なoracleである。

default dictionaryの `ConverterTests` はdirect、roman2kana、AZIK、gradual input、削除、composition separator、must/must-not、誤入力訂正を含む（`Tests/KanaKanjiConverterModuleWithDefaultDictionaryTests/ConverterTests/ConverterTests.swift:42-426`）。`ScenarioTests` は未確定roman suffixの間もprediction表示を安定させるシナリオを持つ（`Tests/KanaKanjiConverterModuleWithDefaultDictionaryTests/ScenarioTests/ScenarioTests.swift:60-95`）。

Zenz prompt testは外部modelなしでtag列を完全一致比較できる（`ZenzPromptBuilderTests.swift:4-157`）。一方、`ZenzaiTests` は `/Library/Input Methods/azooKeyMac.app/...gguf` または環境変数で与える外部N-gramを使い、ない場合は一部をskipする（`ZenzaiTests.swift:55-117`）。実model testはfull conversion、roman/AZIK gradual conversion、予測入力、iOS相当逐次入力、memoization、typo correctionを含む（`ZenzaiTests.swift:267-580`）。

tokenizerは「これは日本語です」から固定ID `[268, 262, 253, 304, 358, 698, 246, 255]` を検証し、代表的な日本語、ASCII、emoji、合成文字でfast/slow path一致を検証する（`EfficientNGramTests.swift:6-13`, `EfficientNGramTests.swift:165-184`）。

### 推論

再実装時は次の順にoracleを使うと原因を分離できる。

1. 入力操作後の `ComposingText`。
2. dictionary byte reader/writerとentry列。
3. nodeとN-best path。
4. candidate data/value/消費範囲。
5. candidate混合後の配列。
6. 確定・学習後の状態。
7. 最後に実modelのprompt/token/logit/prefix loop。

### 未確認事項

- 実GGUFを同梱しないため、clean checkoutだけでZenzaiの意味的test全件を実行できない。
- 性能testの数値はhardware・build mode・warm cacheに依存し、pass/fail thresholdを持たないものもある。
- `DictionaryMock` binaryの生成元provenanceと個別license表示はない。
- 調査環境で `swift test` を試みたが、SwiftPMがb4846 XCFrameworkをdownloadする段階でmacOS keychainのcredential取得エラー `status -128` となったため、test suiteの実行結果は得られていない。本節はtest sourceとfixtureの静的調査結果である。

## 9. コード、辞書、モデル、テストデータのライセンス

### 確認済みの事実

| 資産 | 確認できた条件 | 根拠・注意 |
| --- | --- | --- |
| converterコード・root文書 | MIT、Copyright 2023 Miwa / Ensan | `LICENSE:1-21`。copyright noticeとpermission noticeの保持が必要 |
| default dictionary | Apache License 2.0、Copyright 2024 Miwa / ensan | 固定submodule `4d418525...` には[`LICENSE`](https://github.com/azooKey/azooKey_dictionary_storage/blob/4d418525b090cf49c219819d05a7e3cc2a4346eb/LICENSE)があり、NOTICEはない。license copyとattributionをdictionary packageへ含める |
| emoji source data | MozcはBSD-3-Clause、Unicode/CLDRはUnicode License、独自追加はMIT | emoji submodule `67b822603...` の[`data/README.md`](https://github.com/azooKey/azooKey_emoji_dictionary_storage/blob/67b822603391b01238d7b80b8b61b63f966cf357/data/README.md)。固定`emoji_data.tsv`はMozc `4517e51d...`とbyte一致する。固定dataはUnicode Emoji 17.0を含み、CLDR fileは`SPDX-License-Identifier: Unicode-3.0`を持つ |
| generated EmojiDictionary | 複数sourceの派生 | packageには生成結果だけをcopyするため、Mozc data、Unicode/CLDR、独自追加分のlicense本文とattributionを別途添付する。Mozc自体への製品依存は生じない (`Package.swift:56-67`) |
| EfficientNGram tokenizer data | CC BY-SA 4.0 | 固定原作の[`tokenizer/README.md`](https://github.com/azooKey/AzooKeyKanaKanjiConverter/blob/93766c46e31fa6a18b7ced49dab31337780f6f45/Sources/EfficientNGram/tokenizer/README.md)は`ku-nlp/gpt2-small-japanese-char`由来と明記する。固定fileは原配布元の現行fileと一部異なるため、source URL、license、変更有無を保持し、変更時も同licenseにする |
| Zenz GGUF | repo外。公式DocsはHugging Face URLのみ案内 | `Docs/zenzai.md:19-29`。このrepo内にmodel license本文はない |
| 固定`zenz-v3.2-small-gguf` | Apache-2.0 | [固定commit `c67e03e...`](https://huggingface.co/Miwa-Keita/zenz-v3.2-small-gguf/commit/c67e03e07d215c869f591b274c1631170d3e11fe)のmodel card metadataが宣言する。repositoryは`.gitattributes`、`README.md`、`ggml-model-Q5_K_M.gguf`だけでLICENSEとNOTICEはないため、Apache-2.0本文、repository、commit、filename、Nix hashをmodel packageへ添付する |
| 製品の`pkgs.llama-cpp` | nixpkgsの通常package依存 | shared libraryとしてlinkし、vendor、再包装または静的linkしない。直接配布資産の台帳へ重複登録しない |
| root test source | 個別license markerなし | root MIT配下のsourceとして扱われているが、fixture dataの由来は別途確認が必要 |
| `DictionaryMock` | 個別license/provenanceなし | 本プロジェクトへコピーせず、正常動作テストには本プロジェクトで作成したfixtureを使う |

重要な配布上の注意として、`Package.swift` はdefault dictionary submoduleの `LICENSE`、emoji submoduleのsource data・script・READMEをexcludeし、生成済み `Dictionary` と `EmojiDictionary` だけをresource copyする（`Package.swift:51-69`）。したがって、SwiftPM artifactにresourceを含めることと、必要なlicense・attributionを利用者へ届けることは別の作業である。

配布packageへ添付する条件本文は、[Apache License 2.0](https://www.apache.org/licenses/LICENSE-2.0)、[CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/legalcode)、[Unicode License](https://www.unicode.org/license.txt)および固定sourceの[Mozc LICENSE](https://github.com/google/mozc/blob/4517e51d53063397222adb5512c7ad972b17c181/LICENSE)を一次資料とする。

### 推論

root MITだけを表示して直接配布資産を再配布するのは不十分である。default dictionary、emoji派生data、tokenizerおよびGGUFを個別componentとして管理し、source URL、version/commit、license、attributionおよび変更有無を記録する必要がある。`pkgs.llama-cpp`を含むnixpkgs packageは通常依存として分離する。

### 未確認事項

- default dictionaryのREADMEは`TBW`だけであり、取り込んだ全source corpusのprovenanceは公開されていない。ただし、固定生成物repository自体にはApache-2.0のlicense grantがある。
- 固定GGUF repositoryが示す条件はmodel card metadataのApache-2.0だけで、学習dataや変換元modelの説明はない。配布時はこの明示条件と固定byte hashを記録する。
- emoji submoduleの`data/README.md`にあるUnicode copyright年は固定data fileのheaderより古いため、packageのattributionは固定fileのheaderを基準にする。

## 10. Swift以外で必要部分を再実装する際の難所と追加調査

### 確認済みの事実

原作調査だけから、少なくとも次の難所を確認できる。

1. **Unicode単位の混在**: inputはSwift `Character`、辞書filenameはUTF-16 code unit、entry payloadとZenz prefixはUTF-8 byte、modelはtoken IDを使う（`DictionaryBuilder.swift:156-182`, `ZenzCandidateEvaluator.swift:318-325`）。
2. **入力履歴と表層の非1対1対応**: roman/AZIK/custom table、中間cursor編集、composition separatorを保持する必要がある（`ComposingText.swift:145-374`）。
3. **二重indexラティス**: input countとsurface countを別々に持ち、typoと通常語彙を同じpathへ統合する（`Lattice.swift:15-110`）。
4. **辞書binary**: LOUDS、character ID、2048 shard、variable payload、CID sparse、MID denseを互換に読む必要がある。
5. **数値一致**: `PValue` はiOS/tvOSでFloat16、それ以外でFloat32であり、負costと加算順が順位へ影響する（`Kana2Kanji.swift:9-15`）。
6. **候補合成policy**: 全文・予測・shortcut・英語・文節・単語・特殊候補のdedupと順位調整が外部挙動になる（`KanaKanjiConverter.swift:850-1084`）。
7. **状態ful最適化**: 初回、無変更、文節確定後、末尾差分の別経路とprediction安定化cacheがある。
8. **学習永続化**: temporary/long-term dictionaryと `.2` / `.pause` recoveryを再現する必要がある。
9. **Zenzaiの閉loop**: prompt/token/logitだけでなく、modelの修正prefixを辞書制約探索へ戻す必要がある。
10. **runtime version依存**: llama.cpp C API、KV cache、backend、thread/cacheの挙動が性能と結果に関わる。

### 推論

「同じ辞書を読める」「同じtop-1が出る」「Zenz modelを呼べる」は、それぞれ別の互換レベルである。必要部分を選ぶ際は、少なくとも次の適合境界を別々に定義すべきである。

- input-state parity
- dictionary-format parity
- lattice/path parity
- candidate-set parity
- candidate-order parity
- commit/learning parity
- Zenz prompt/token parity
- Zenz final-result parity
- latency/memory parity

### 未確認事項・追加調査が必要な点

- default dictionary全sourceのprovenanceとgeneration pipeline。
- `mm.binary` / `cb/*.binary` のportableなendianness・alignment specification。
- malformed辞書のerror contract。
- GGUF modelのtokenizer/configとquantization条件。
- llama.cpp b4846 archiveのbuild flags、ABIおよび各platformのlibrary配置。これは将来の原作比較環境だけの確認事項である。
- 原作製品が使うoption値、ライブ変換のUI event timing、確定・学習の呼出順。
- 全候補順位を固定するgolden vector。既存testは強力だが、すべての公開optionの直積を網羅しない。
- Apple SpellCheckerと非Apple platformの補完差。
- Float16/Float32の差を許容するか、platformごとにbit-level一致を要求するか。

## 再実装の同等性を評価する境界

この節は機能範囲を決めるものではない。実装対象として選ばれた機能を、固定した原作に対してどの境界で比較できるかを整理する。

### 確認済みの事実

辞書変換のscoreは、単純な「単語cost」1個ではない。

- 辞書entryの実効値は `min(0, baseValue + adjust)` であり、正値へはならない（`DicdataElement.swift:48-64`）。
- ラティス先頭のnodeではBOSからの累積値にentry値とCID連接値を加える。それ以外のnodeは、前段から当該nodeへ伝播するときにCID連接値を加え、当該nodeの処理時にentry値を加える（`FullInputProcessing.swift:72-98`, `FullInputProcessing.swift:110-132`）。N-bestへの挿入では累積値が大きいものを前へ置く。
- 最終candidateでは文節間のMID連接値も加える（`Kana2Kanji.swift:27-48`）。CID値とMID値は「確率の対数」「意味連接確率の対数」と説明される（`DicdataStore.swift:911-954`）。
- 辞書entry payloadのscoreはFloat32で格納され、読込み時にplatform依存の `PValue` へ変換される（`DictionaryBuilder.swift:275-311`, `extension LOUDS.swift:107-114`）。`PValue` はiOS/tvOSでFloat16、それ以外でFloat32である（`Kana2Kanji.swift:9-15`）。

Zenzaiの判定も、最終文字列だけでなく途中の数値比較へ依存する。

- candidate評価はllama.cppの `Float` logitsを語彙全体で走査し、現在値より厳密に大きいlogitだけをmaximumへ採用する。candidate tokenがmaximumでなければ、そのtokenへ学習priorityを加えた値との比較を経て、修正prefixを返す（`ZenzCandidateEvaluator.swift:210-325`）。通過時のscoreは各tokenのmaximum logitの加算である（`ZenzCandidateEvaluator.swift:347-375`）。
- greedy decodeも語彙順にlogitを走査し、厳密な `<` の場合だけmaximum tokenを更新するため、数値上の同値では先に走査したtokenが残る（`ZenzPureGreedyDecoder.swift:19-35`）。
- 原作はcontextを512 token、batch 512、microbatch 64、flash attention有効に設定する。thread数は実行機のactive processor数と、Darwinではperformance core数から自動決定される（`ZenzContext.swift:342-420`）。
- `ZenzaiCPU` はGPU layer、device list、KQV offloadをCPU専用にするが、`Zenzai` はllama.cppの既定device選択を使う（`ZenzContext.swift:273-303`, `ZenzContext.swift:433-446`）。したがって、同じGGUFでもtraitと実行backendは別条件である。

### 評価方針としての推論

厳密な比較では、少なくとも原作コミット、対象platform、SwiftPM trait、辞書submodule commitと生成物、GGUFのbyte hash、Zenz config、変換option、left/right context、入力操作列、session・cache・学習状態を固定しなければならない。そのうえで、選択した機能ごとに次のtraceを比較するのが適切である。

1. 各insert/delete/cursor move/確定操作後のinput履歴、surface、`convertTarget`、cursor、残入力。
2. 参照した辞書entryとCID/MID、entry値・adjust・CID連接値・MID連接値、累積値、採用path。
3. request上限内で生成されたcandidateの文字列、順序、消費範囲、prediction/typo等の属性。top-1だけではなく、対象機能が公開する候補列を比較する。
4. candidate選択、部分確定、学習後の状態遷移と、その後の候補列。
5. ZenzaiではpromptのUTF-8 bytes、token ID列、model世代別tag、left/right context、各loopのdraft、maximum token、修正prefix、制約付き再探索結果、最終候補列。

この評価では「自然な候補か」「他IMEより優れているか」「独自改善で正解率が上がったか」は判定しない。原作と異なる候補が主観的には良く見えても、原作再現という評価では差分として扱う。

### 完全一致が難しい境界と未確認事項

- **Float16とFloat32**: 原作自体がplatformによって辞書scoreの計算型を変える。同一入力でも丸めと加算結果が一致する保証はないため、まず同じ原作platformの結果をoracleにする必要がある。異なるplatform間のbit-level一致は、公式リポジトリでは要求も許容差も定義されていない。
- **llama.cpp backend**: `Zenzai` と `ZenzaiCPU` ではdevice/offload条件が異なり、thread数も実行環境で変わる。公式リポジトリにはCPU・Metal・その他backend間のlogit完全一致保証や数値許容差がない。原作側は同じ`b4846`、GGUF、trait、backend、device、thread条件を揃え、製品側は`flake.lock`が固定する`pkgs.llama-cpp`、GGUF、backend、device、thread条件を揃えたうえで、両者を比較する実測oracleが必要である。
- **境界値での分岐**: Zenzaiはlogitの大小でmaximum tokenや修正prefixを決めるため、わずかな数値差でもargmaxが入れ替わる近接値では、最終候補まで分岐し得る（`ZenzCandidateEvaluator.swift:285-325`）。prompt bytesとtoken IDは完全一致、logitは絶対・相対誤差とargmax margin、候補列は完全な順序差として別々に記録するのが妥当である。ただし、採用すべき誤差閾値は原作に規定がない。
- **同scoreの順序**: 通常N-bestは `>=` を含む挿入条件、Zenzaiの一部top-kはscoreだけを比較するheapを使う（`FullInputProcessing.swift:120-132`, `ZenzCandidateEvaluator.swift:190-220`）。完全同値時の順序をplatform横断の公開契約とする記述は確認できないため、固定oracleで実測し、同値群を識別しておく必要がある。
- **cache状態**: ZenzaiはKV、draft、resolved conversion、prompt tokenのcacheを持つ（`ZenzContext.swift:262-265`, `ZenzContext.swift:342-347`）。cold/fullとwarm/incrementalを別scenarioとして固定し、双方が同じ最終候補になるかも原作側で先に記録する必要がある。

数値差を許容する場合でも、それを理由に入力状態、辞書costの符号・加算項・順位方向、prompt/token、prefix loopの差をまとめて許容してはならない。数値誤差と意味的な実装差は別々に判定する必要がある。

## 実装対象の機能範囲を決めるための材料

ここでは範囲を決定せず、依存関係と検証可能性だけを示す。

### 機能単位の比較材料

| 機能単位 | 直接必要になるもの | 原作testによる検証 | 主な難所・切離し条件 |
| --- | --- | --- | --- |
| 直接かな入力 + 通常変換 | static dictionary、LOUDS、ラティス、candidate | 強い | input mappingの最小形。prediction/learning/Zenzを切離せる |
| roman2kana | `ComposingText`、input table、dual index | 強い | 未確定suffix、中間cursor、入力数と表層数の差 |
| AZIK・かなJIS/US・custom table | mapped table parser/trie | 強い | table syntax、shift key、composition separator |
| 文節確定・複数session | composing count、session state、再変換 | 強い | UIの確定protocolと状態分離 |
| 入力中辞書予測 | prefix辞書、prediction mixing/cache | 強い | 通常候補との順位、逐次安定性 |
| 確定後予測 | `pc_*.csv`、compound lookup、emoji | 中〜強 | zero-hintとreplacement、別license資産 |
| 特殊候補 | providerごとの純粋処理 | 強い | coreから独立しやすいが地域・日付挙動を持つ |
| user dictionary | dynamic entryまたはLOUDS user files | 強い | metadataとcache invalidation |
| 学習 | memory dictionary、保存・recovery | 強い | 永続化、decay、forget、migration |
| ライブ変換 | 通常候補から完全一致bestを選ぶ | engine側は中 | 機能は薄いがUI timingとlatencyはrepo外 |
| classic typo correction | typo rule + dictionary同時枝刈り | 強い | dual indexと候補爆発抑制 |
| Zenzai候補評価 | GGUF、llama.cpp、prompt、prefix再探索 | modelなしでは部分的 | runtime/model/license/性能が別componentになる |
| Zenz予測入力 | v3 prompt、greedy生成、通常変換fallback | model依存 | experimental flag、roman possible-nexts |
| LM typo correction | Zenzまたは4 Marisa N-gram、beam search | 一部model依存 | experimental API、追加data asset |
| CLI | converter APIとArgumentParser | 中 | engineのoracle runnerとして有用だがIME機能ではない |

### 依存順序として確認できること

```text
入力操作・InputTable
  -> ComposingText (input / surface / cursor)
  -> Dictionary reader + cost tables
  -> Dual-indexed lattice + N-best path
  -> Candidate construction and ordering
  -> Candidate selection / partial completion
  -> User dictionary and learning

Zenzaiは Dictionary/Lattice/Candidate の上に乗り、
GGUF + tokenizer + llama.cpp logits
  -> model prefix constraint
  -> constrained lattice search
というloopを追加する。
```

### 判断前に数値化できる軸

- 対応する入力style数。
- 直接入力、roman逐次入力、削除、中間cursor、部分確定のgolden scenario数。
- 対象機能が返す候補列について、request上限内の生成候補と完全な順序を比較できるgolden scenario数。
- 固定したdefault dictionaryをoracle用に使う方法と、配布物へ含めるかを分離して扱えるか。
- 1 requestのcold/warm latency、長文逐次入力のp50/p90、peak memory。
- 学習・user dictionaryの永続互換を要求するか。
- Zenzaiなし、Zenzai候補評価のみ、予測入力・LM typoまでのどこを対象とするか。
- Zenzaiを対象にする場合、原作側のGGUF hash、b4846、trait、backend、device、thread、cold/warm cacheと、製品側の固定`pkgs.llama-cpp`および対応する実行条件をそれぞれ固定したoracleを用意できるか。
- 直接配布assetのlicense・attribution添付をrelease gateとして検証する方法。

### 最後まで未決定のまま残すべき事項

原作調査だけでは機能範囲を決めず、`spec.md`で固定原作のconverter機能をすべて実装すると決定した。default dictionary、固定GGUF、必要なtokenizerおよび絵文字は、本文で確認したlicense・attributionを添付して配布する。
