# beankey（仮称）

azooKey Desktopの操作感をNixOSとFcitx5で再現する、RustおよびC++製の日本語入力エンジン

現在は開発中で、NixOSとFcitx5上で互換性を検証中

使用モデル: [Miwa-Keita/zenz-v3.2-small-gguf](https://huggingface.co/Miwa-Keita/zenz-v3.2-small-gguf)

## 設定

flakeの`nixosModules.default`をNixOS構成へ追加したうえで、`programs.beankey`を設定

設定例

```nix
programs.beankey = {
  enable = true;
  useBeankeyTheme = true;

  conversion = {
    typeBackslash = false;
    typeHalfSpace = true;
    optionDirectFullWidthInput = false;
    punctuationStyle = "kuten_and_toten";
  };

  zenz = {
    inferenceLimit = 4;
    profile = "学生";
    topic = "プログラミング";
    preference = "カタカナ優先";
  };
};
```

### 変換 (`conversion`)

| オプション | 初期値 | 説明 |
| --- | --- | --- |
| `inputStyle` | `"roman_to_kana"` | 入力方式（`"direct"`、`"roman_to_kana"`、`"azik"`、`"kana_jis"`、`"kana_us"`、`"custom"`から選択） |
| `customInputTable` | `null` | `inputStyle = "custom"`で使う登録済み入力テーブル名 |
| `keyboardLanguage` | `"japanese"` | 外国語補完に使うキーボード言語（`"none"`、`"japanese"`、`"english_us"`、`"greek"`から選択） |
| `candidateCount` | `10` | 一度に要求する変換候補数 |
| `japanesePrediction` | `"disabled"` | 日本語予測の動作（`"automatic"`、`"manual"`、`"disabled"`から選択） |
| `foreignPrediction` | `"disabled"` | 英語・ギリシャ語予測の動作（`"automatic"`、`"manual"`、`"disabled"`から選択） |
| `fullWidthRoman` | `true` | 全角英数候補の生成 |
| `halfWidthKana` | `false` | 半角カナ候補の生成 |
| `typography` | `false` | タイポグラフィ候補の生成 |
| `typoCorrection` | `"automatic"` | 辞書による入力訂正（`"enabled"`、`"automatic"`、`"disabled"`から選択） |
| `liveConversion` | `true` | 入力中の最良完全一致候補をプリエディットへ表示 |
| `typeBackslash` | `false` | `true`で円記号キーから`¥`の代わりに`\`を入力（Alt押下中は結果を反転） |
| `typeHalfSpace` | `false` | `true`でSpaceを半角、Shift+Spaceを全角に設定（`false`では逆） |
| `optionDirectFullWidthInput` | `false` | `true`で変換中でないときにAltまたはShift+Altから英数字・記号を直接全角入力 |
| `punctuationStyle` | `"kuten_and_toten"` | コンマキーとピリオドキーの句読点の組み合わせ（Alt押下中は和文・欧文の記号を反転） |
| `userDictionary` | `null` | JSONユーザー辞書の絶対パス |
| `userDictionaryDirectory` | `null` | azooKey形式ユーザー辞書ディレクトリの絶対パス |
| `customInputTables` | `{ }` | 入力テーブル名とカスタムJSONファイルの対応 |

`punctuationStyle`で指定可能な値

| 値 | コンマキー | ピリオドキー |
| --- | --- | --- |
| `"kuten_and_toten"` | `、` | `。` |
| `"kuten_and_comma"` | `，` | `。` |
| `"period_and_toten"` | `、` | `．` |
| `"period_and_comma"` | `，` | `．` |

### 学習 (`learning`)

| オプション | 初期値 | 説明 |
| --- | --- | --- |
| `mode` | `"input_and_output"` | 学習データの扱い（`"input_and_output"`は読み書き、`"only_output"`は読み取りのみ、`"nothing"`は無効） |
| `maxCount` | `65536` | 永続化する学習レコード数の上限 |

### Zenzai推論 (`zenz`)

| オプション | 初期値 | 説明 |
| --- | --- | --- |
| `inferenceLimit` | `5` | 推論上限 |
| `profile` | `null` | 変換プロフィール |
| `topic` | `null` | 話題/分野 |
| `style` | `null` | 文体 |
| `preference` | `null` | 表記の優先 |
| `richCandidates` | `false` | 詳細候補を要求 |
| `predictiveInput` | `false` | 次入力予測を有効化 |
| `enableAlignmentSeparator` | `true` | プロンプトへアラインメント区切りを追加 |
| `personalization` | `null` | EfficientNGramによる個人化設定（`baseNgram`、`personalNgram`、`alpha`を指定） |

### 言語モデル入力訂正 (`lmTypo`)

| オプション | 初期値 | 説明 |
| --- | --- | --- |
| `enabled` | `false` | LM入力訂正を有効化 |
| `languageModel` | `"zenz"` | 使用する言語モデル（`"zenz"`または`"ngram"`から選択） |
| `ngram` | `null` | `languageModel = "ngram"`で使うEfficientNGram設定（`prefix`、`n`、`discount`を指定） |
| `beamSize` | `32` | 探索時のビーム幅 |
| `topK` | `64` | 各ステップで探索するトークン数の上限 |
| `candidateCount` | `5` | 返す入力訂正候補数 |
| `maxSteps` | `null` | デコードのステップ数の上限（`null`では自動決定） |
| `substitutionCost` | `2.0` | 文字置換のチャネルコスト |
| `deletionCost` | `3.0` | 文字削除のチャネルコスト |
| `transpositionCost` | `2.0` | 隣接文字入れ替えのチャネルコスト |

## 設計文書

- [仕様書](docs/spec.md)
- [アーキテクチャ](docs/architecture.md)
