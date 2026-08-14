{ self }:
{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.programs.beankey;
  inherit (lib)
    mkEnableOption
    mkIf
    mkOption
    optionalAttrs
    types
    ;
  system = pkgs.stdenv.hostPlatform.system;
  packages = self.packages.${system};

  predictionType = types.enum [
    "automatic"
    "manual"
    "disabled"
  ];
  inputStyleType = types.enum [
    "direct"
    "roman_to_kana"
    "azik"
    "kana_jis"
    "kana_us"
    "custom"
  ];
  optionalString =
    description:
    mkOption {
      type = types.nullOr types.str;
      default = null;
      inherit description;
    };
  absolutePath =
    description:
    mkOption {
      type = types.nullOr types.str;
      default = null;
      inherit description;
    };

  personalizationConfig = types.submodule {
    options = {
      baseNgram = mkOption {
        type = types.str;
        description = "Absolute prefix for the base EfficientNGram files.";
      };
      personalNgram = mkOption {
        type = types.str;
        description = "Absolute prefix for the personal EfficientNGram files.";
      };
      alpha = mkOption {
        type = types.float;
        default = 0.5;
        description = "Interpolation strength for the personal language model.";
      };
    };
  };

  typoNgramConfig = types.submodule {
    options = {
      prefix = mkOption {
        type = types.str;
        description = "Absolute prefix for the EfficientNGram files.";
      };
      n = mkOption {
        type = types.ints.positive;
        default = 5;
        description = "N-gram order.";
      };
      discount = mkOption {
        type = types.float;
        default = 0.75;
        description = "N-gram discount value.";
      };
    };
  };

  conversionConfig = {
    input_style = cfg.conversion.inputStyle;
    n_best = cfg.conversion.candidateCount;
    japanese_prediction = cfg.conversion.japanesePrediction;
    foreign_prediction = cfg.conversion.foreignPrediction;
    full_width_roman = cfg.conversion.fullWidthRoman;
    half_width_kana = cfg.conversion.halfWidthKana;
    typography = cfg.conversion.typography;
    typo_correction = cfg.conversion.typoCorrection;
    live_conversion = cfg.conversion.liveConversion;
    custom_input_tables = lib.mapAttrs (_: value: toString value) cfg.conversion.customInputTables;
  }
  // optionalAttrs (cfg.conversion.customInputTable != null) {
    custom_input_table = cfg.conversion.customInputTable;
  }
  // optionalAttrs (cfg.conversion.userDictionary != null) {
    user_dictionary = cfg.conversion.userDictionary;
  }
  // optionalAttrs (cfg.conversion.userDictionaryDirectory != null) {
    user_dictionary_directory = cfg.conversion.userDictionaryDirectory;
  };

  zenzConfig = {
    inference_limit = cfg.zenz.inferenceLimit;
    rich_candidates = cfg.zenz.richCandidates;
    predictive_input = cfg.zenz.predictiveInput;
    enable_alignment_separator = cfg.zenz.enableAlignmentSeparator;
  }
  // optionalAttrs (cfg.zenz.profile != null) { inherit (cfg.zenz) profile; }
  // optionalAttrs (cfg.zenz.topic != null) { inherit (cfg.zenz) topic; }
  // optionalAttrs (cfg.zenz.style != null) { inherit (cfg.zenz) style; }
  // optionalAttrs (cfg.zenz.preference != null) { inherit (cfg.zenz) preference; }
  // optionalAttrs (cfg.zenz.personalization != null) {
    personalization = {
      base_ngram = cfg.zenz.personalization.baseNgram;
      personal_ngram = cfg.zenz.personalization.personalNgram;
      inherit (cfg.zenz.personalization) alpha;
    };
  };

  lmTypoConfig = {
    inherit (cfg.lmTypo) enabled;
    language_model = cfg.lmTypo.languageModel;
    beam_size = cfg.lmTypo.beamSize;
    top_k = cfg.lmTypo.topK;
    n_best = cfg.lmTypo.candidateCount;
    substitution_cost = cfg.lmTypo.substitutionCost;
    deletion_cost = cfg.lmTypo.deletionCost;
    transposition_cost = cfg.lmTypo.transpositionCost;
  }
  // optionalAttrs (cfg.lmTypo.maxSteps != null) {
    max_steps = cfg.lmTypo.maxSteps;
  }
  // optionalAttrs (cfg.lmTypo.ngram != null) {
    ngram = {
      inherit (cfg.lmTypo.ngram) discount n;
      prefix = cfg.lmTypo.ngram.prefix;
      tokenizer = "${packages.tokenizer}/share/beankey/tokenizer/tokenizer.json";
    };
  };

  configFile = (pkgs.formats.toml { }).generate "beankey-config.toml" {
    dictionary = "${packages.dictionary}/share/beankey/dictionary";
    model = "${packages.model}/share/beankey/model/ggml-model-Q5_K_M.gguf";
    emoji_dictionary = "${packages.emoji}/share/beankey/emoji/emoji_all_E17.0.txt";
    llama_backend_directory = "${packages.daemon.llamaCpp}/bin";
    runtime_socket = "beankey/daemon.sock";
    hunspell = {
      english_dictionary = "${packages.daemon.hunspellEnglish}/share/hunspell/en_US";
      greek_dictionary = "${packages.daemon.hunspellGreek}/share/hunspell/el_GR";
    };
    conversion = conversionConfig;
    zenz = zenzConfig;
    lm_typo = lmTypoConfig;
    inference = {
      context_size = 512;
      batch_size = 512;
      micro_batch_size = 64;
      flash_attention = true;
    };
  };
in
{
  options.programs.beankey = {
    enable = mkEnableOption "beankey kana-kanji conversion for Fcitx5";

    conversion = {
      inputStyle = mkOption {
        type = inputStyleType;
        default = "roman_to_kana";
        description = "Default input style.";
      };
      customInputTable = optionalString "Name of the default custom input table.";
      candidateCount = mkOption {
        type = types.ints.positive;
        default = 10;
        description = "Number of conversion candidates to request.";
      };
      japanesePrediction = mkOption {
        type = predictionType;
        default = "automatic";
        description = "Japanese prediction mode.";
      };
      foreignPrediction = mkOption {
        type = predictionType;
        default = "automatic";
        description = "English and Greek prediction mode.";
      };
      fullWidthRoman = mkOption {
        type = types.bool;
        default = false;
        description = "Whether to generate full-width Roman candidates.";
      };
      halfWidthKana = mkOption {
        type = types.bool;
        default = false;
        description = "Whether to generate half-width Kana candidates.";
      };
      typography = mkOption {
        type = types.bool;
        default = false;
        description = "Whether to generate typography variants.";
      };
      typoCorrection = mkOption {
        type = types.enum [
          "enabled"
          "automatic"
          "disabled"
        ];
        default = "automatic";
        description = "Dictionary typo correction mode.";
      };
      liveConversion = mkOption {
        type = types.bool;
        default = false;
        description = "Whether to display the best exact candidate while composing.";
      };
      userDictionary = absolutePath "Absolute path to a JSON user dictionary.";
      userDictionaryDirectory = absolutePath "Absolute path to an azooKey user dictionary directory.";
      customInputTables = mkOption {
        type = types.attrsOf types.path;
        default = { };
        description = "Named custom input table JSON files.";
      };
    };

    zenz = {
      inferenceLimit = mkOption {
        type = types.ints.unsigned;
        default = 10;
        description = "Maximum Zenz prefix correction attempts.";
      };
      richCandidates = mkOption {
        type = types.bool;
        default = false;
        description = "Whether to request rich Zenz candidates.";
      };
      predictiveInput = mkOption {
        type = types.bool;
        default = false;
        description = "Whether to enable Zenz next-input prediction.";
      };
      profile = optionalString "Zenz user profile condition.";
      topic = optionalString "Zenz topic condition.";
      style = optionalString "Zenz style condition.";
      preference = optionalString "Zenz preference condition.";
      enableAlignmentSeparator = mkOption {
        type = types.bool;
        default = false;
        description = "Whether to include alignment separators in Zenz prompts.";
      };
      personalization = mkOption {
        type = types.nullOr personalizationConfig;
        default = null;
        description = "Optional EfficientNGram personalization models.";
      };
    };

    lmTypo = {
      enabled = mkOption {
        type = types.bool;
        default = false;
        description = "Whether to expose experimental LM typo corrections.";
      };
      languageModel = mkOption {
        type = types.enum [
          "zenz"
          "ngram"
        ];
        default = "zenz";
        description = "Language model used by LM typo correction.";
      };
      ngram = mkOption {
        type = types.nullOr typoNgramConfig;
        default = null;
        description = "EfficientNGram model used for LM typo correction.";
      };
      beamSize = mkOption {
        type = types.ints.positive;
        default = 32;
        description = "LM typo correction beam size.";
      };
      topK = mkOption {
        type = types.ints.positive;
        default = 64;
        description = "Token branching limit for LM typo correction.";
      };
      candidateCount = mkOption {
        type = types.ints.positive;
        default = 5;
        description = "Number of LM typo correction candidates.";
      };
      maxSteps = mkOption {
        type = types.nullOr types.ints.positive;
        default = null;
        description = "Optional decoding step limit for LM typo correction.";
      };
      substitutionCost = mkOption {
        type = types.float;
        default = 2.0;
        description = "LM typo substitution channel cost.";
      };
      deletionCost = mkOption {
        type = types.float;
        default = 3.0;
        description = "LM typo deletion channel cost.";
      };
      transpositionCost = mkOption {
        type = types.float;
        default = 2.0;
        description = "LM typo transposition channel cost.";
      };
    };
  };

  config = mkIf cfg.enable {
    assertions = [
      {
        assertion =
          cfg.conversion.inputStyle != "custom"
          || (
            cfg.conversion.customInputTable != null
            && builtins.hasAttr cfg.conversion.customInputTable cfg.conversion.customInputTables
          );
        message = "programs.beankey.conversion.customInputTable must name a registered table when inputStyle is custom";
      }
      {
        assertion =
          cfg.zenz.personalization == null
          || (cfg.zenz.personalization.alpha >= 0.0 && cfg.zenz.personalization.alpha <= 1.0);
        message = "programs.beankey.zenz.personalization.alpha must be between 0 and 1";
      }
      {
        assertion = cfg.lmTypo.languageModel != "ngram" || cfg.lmTypo.ngram != null;
        message = "programs.beankey.lmTypo.ngram is required when languageModel is ngram";
      }
      {
        assertion = builtins.all (value: value == null || lib.hasPrefix "/" value) [
          cfg.conversion.userDictionary
          cfg.conversion.userDictionaryDirectory
          (if cfg.zenz.personalization == null then null else cfg.zenz.personalization.baseNgram)
          (if cfg.zenz.personalization == null then null else cfg.zenz.personalization.personalNgram)
          (if cfg.lmTypo.ngram == null then null else cfg.lmTypo.ngram.prefix)
        ];
        message = "programs.beankey runtime data paths must be absolute";
      }
    ];

    i18n.inputMethod = {
      enable = true;
      type = "fcitx5";
      fcitx5.addons = [ packages.fcitx5-addon ];
    };
    environment.systemPackages = [ packages.daemon ];
    environment.etc."beankey/config.toml".source = configFile;
  };
}
