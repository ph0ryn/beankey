{
  description = "Development environment for beankey";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    self.submodules = true;
  };

  outputs =
    { self, nixpkgs, ... }:
    let
      systems = [
        "aarch64-darwin"
        "aarch64-linux"
        "x86_64-linux"
      ];
      forAllSystems = nixpkgs.lib.genAttrs systems;
      pkgsFor = system: import nixpkgs { inherit system; };
      developmentPackages = pkgs: [
        pkgs.cargo
        pkgs.clang
        pkgs.clang-tools
        pkgs.clippy
        pkgs.cmake
        pkgs.git
        pkgs.ninja
        pkgs.pkg-config
        pkgs.protobuf
        pkgs.rust-analyzer
        pkgs.rustc
        pkgs.rustfmt
      ];
      sourceFor =
        pkgs: roots:
        pkgs.lib.cleanSourceWith {
          src = ./.;
          filter =
            path: _:
            let
              relative = pkgs.lib.removePrefix "${toString ./.}/" (toString path);
            in
            builtins.any (root: relative == root || pkgs.lib.hasPrefix "${root}/" relative) roots;
        };
      beankeyModelSource =
        pkgs:
        pkgs.fetchurl {
          url = "https://huggingface.co/Miwa-Keita/zenz-v3.2-small-gguf/resolve/c67e03e07d215c869f591b274c1631170d3e11fe/ggml-model-Q5_K_M.gguf";
          hash = "sha256-KcIj1MIzJ7gP0T67WrJVUFekYxeZfV2jkVhP++8NtnM=";
        };
      beankeyUpstream =
        pkgs:
        pkgs.fetchFromGitHub {
          owner = "azooKey";
          repo = "AzooKeyKanaKanjiConverter";
          rev = "93766c46e31fa6a18b7ced49dab31337780f6f45";
          hash = "sha256-euGFHvSc0MfXpXg+0GdwgLpCbV9NmM8B/e/wHsKrXDE=";
        };
      beankeyTokenizer =
        pkgs:
        let
          attribution = pkgs.writeText "beankey-tokenizer-attribution" ''
            Asset: EfficientNGram tokenizer data
            Source: azooKey/AzooKeyKanaKanjiConverter
            Fixed revision: 93766c46e31fa6a18b7ced49dab31337780f6f45
            Original distribution: ku-nlp/gpt2-small-japanese-char
            License: Creative Commons Attribution-ShareAlike 4.0 International
            Changes by beankey: none; files are copied from the fixed azooKey revision.
          '';
          license = pkgs.fetchurl {
            url = "https://creativecommons.org/licenses/by-sa/4.0/legalcode.txt";
            hash = "sha256-KKlSnH0LtNxR9L9cEWo9Fu8kegUvdZFGZ2jd9WP9HPU=";
          };
        in
        pkgs.runCommand "beankey-zenz-tokenizer"
          {
            meta.license = pkgs.lib.licenses.cc-by-sa-40;
          }
          ''
            mkdir -p "$out/share/beankey/tokenizer"
            mkdir -p "$out/share/licenses/beankey-tokenizer"
            cp -r ${beankeyUpstream pkgs}/Sources/EfficientNGram/tokenizer/. \
              "$out/share/beankey/tokenizer/"
            cp ${license} "$out/share/licenses/beankey-tokenizer/CC-BY-SA-4.0.txt"
            cp ${attribution} "$out/share/licenses/beankey-tokenizer/ATTRIBUTION"
          '';
      beankeyDictionary =
        pkgs:
        let
          attribution = pkgs.writeText "beankey-dictionary-attribution" ''
            Asset: azooKey dictionary storage
            Source: azooKey/azooKey_dictionary_storage
            Fixed revision: 4d418525b090cf49c219819d05a7e3cc2a4346eb
            License: Apache License 2.0
            Copyright 2024 Miwa / ensan
            Changes by beankey: none; generated dictionary files are copied directly.
            Upstream NOTICE: none at the fixed revision.
          '';
        in
        pkgs.runCommand "beankey-dictionary"
          {
            meta.license = pkgs.lib.licenses.asl20;
          }
          ''
            mkdir -p "$out/share/beankey/dictionary"
            mkdir -p "$out/share/licenses/beankey-dictionary"
            cp -r ${./data/azooKey_dictionary_storage/Dictionary}/. \
              "$out/share/beankey/dictionary/"
            cp ${./data/azooKey_dictionary_storage/LICENSE} \
              "$out/share/licenses/beankey-dictionary/LICENSE"
            cp ${attribution} "$out/share/licenses/beankey-dictionary/ATTRIBUTION"
          '';
      beankeyEmoji =
        pkgs:
        let
          mozcLicense = pkgs.fetchurl {
            url = "https://raw.githubusercontent.com/google/mozc/4517e51d53063397222adb5512c7ad972b17c181/LICENSE";
            hash = "sha256-RM3ZI7keqRmSk6vswnYscMh9vx5YHAJ6lMQWNo0aZIw=";
          };
          unicodeLicense = pkgs.fetchurl {
            url = "https://www.unicode.org/license.txt";
            hash = "sha256-56k7AJVlz85VkZo4FDesTbiD6dohJvoouR0ScyvFPZY=";
          };
          attribution = pkgs.writeText "beankey-emoji-attribution" ''
            Asset: generated azooKey emoji dictionary for Unicode Emoji 17.0
            Source: azooKey/azooKey_emoji_dictionary_storage
            Fixed revision: 67b822603391b01238d7b80b8b61b63f966cf357
            Packaged file: EmojiDictionary/emoji_all_E17.0.txt
            Changes by beankey: none; the generated dictionary is copied directly.

            Mozc-derived data:
            Source revision: google/mozc 4517e51d53063397222adb5512c7ad972b17c181
            Copyright 2010-2018, Google Inc. All rights reserved.
            License: BSD 3-Clause

            Unicode Emoji and CLDR-derived data:
            Copyright 2022 Unicode, Inc. for emoji data.
            Copyright 1991-2023 Unicode, Inc. for CLDR data.
            License: Unicode License V3

            azooKey additional emoji data:
            Copyright 2023 Miwa / Ensan
            License: MIT
          '';
        in
        pkgs.runCommand "beankey-emoji-dictionary"
          {
            meta.license = with pkgs.lib.licenses; [
              bsd3
              unicode-30
              mit
            ];
          }
          ''
            mkdir -p "$out/share/beankey/emoji"
            mkdir -p "$out/share/licenses/beankey-emoji"
            cp ${./data/azooKey_emoji_dictionary_storage/EmojiDictionary/emoji_all_E17.0.txt} \
              "$out/share/beankey/emoji/emoji_all_E17.0.txt"
            cp ${./data/azooKey_emoji_dictionary_storage/data/README.md} \
              "$out/share/licenses/beankey-emoji/UPSTREAM-DATA.md"
            cp ${mozcLicense} "$out/share/licenses/beankey-emoji/BSD-3-Clause.txt"
            cp ${unicodeLicense} "$out/share/licenses/beankey-emoji/Unicode-License-V3.txt"
            cp ${beankeyUpstream pkgs}/LICENSE \
              "$out/share/licenses/beankey-emoji/MIT.txt"
            cp ${attribution} "$out/share/licenses/beankey-emoji/ATTRIBUTION"
          '';
      beankeyModel =
        pkgs:
        let
          license = pkgs.fetchurl {
            url = "https://www.apache.org/licenses/LICENSE-2.0.txt";
            hash = "sha256-z8d0m5b2O9McPEK1xHG/dWgUBT6EfBDz6wA0F7xSPTA=";
          };
          attribution = pkgs.writeText "beankey-model-attribution" ''
            Asset: zenz-v3.2-small GGUF model
            Source: Miwa-Keita/zenz-v3.2-small-gguf
            Fixed revision: c67e03e07d215c869f591b274c1631170d3e11fe
            File: ggml-model-Q5_K_M.gguf
            Source hash: sha256-KcIj1MIzJ7gP0T67WrJVUFekYxeZfV2jkVhP++8NtnM=
            License: Apache License 2.0, as declared by the fixed model card metadata.
            Changes by beankey: none; the fetched bytes are copied directly.
            Upstream LICENSE and NOTICE: none at the fixed revision.
          '';
        in
        pkgs.runCommand "beankey-zenz-v3.2-small-gguf"
          {
            meta.license = pkgs.lib.licenses.asl20;
          }
          ''
            mkdir -p "$out/share/beankey/model"
            mkdir -p "$out/share/licenses/beankey-model"
            cp ${beankeyModelSource pkgs} \
              "$out/share/beankey/model/ggml-model-Q5_K_M.gguf"
            cp ${license} \
              "$out/share/licenses/beankey-model/Apache-2.0.txt"
            cp ${attribution} "$out/share/licenses/beankey-model/ATTRIBUTION"
          '';
      beankeyDaemon =
        pkgs:
        pkgs.rustPlatform.buildRustPackage {
          pname = "beankey-daemon";
          version = "0.1.0";
          src = sourceFor pkgs [
            "Cargo.lock"
            "Cargo.toml"
            "LICENSE"
            "crates"
            "data"
            "proto"
          ];
          cargoLock.lockFile = ./Cargo.lock;
          cargoBuildFlags = [
            "--package"
            "beankey-daemon"
          ];
          cargoTestFlags = [
            "--package"
            "beankey-daemon"
          ];
          nativeBuildInputs = [
            pkgs.pkg-config
            pkgs.protobuf
          ];
          buildInputs = [
            pkgs.hunspell
            pkgs.llama-cpp
            pkgs.marisa
          ];
          BEANKEY_TEST_EN_US_DICTIONARY = "${pkgs.hunspellDicts.en_US}/share/hunspell/en_US";
          BEANKEY_TEST_EL_GR_DICTIONARY = "${pkgs.hunspellDicts.el_GR}/share/hunspell/el_GR";
          BEANKEY_TEST_ZENZ_TOKENIZER = "${beankeyTokenizer pkgs}/share/beankey/tokenizer/tokenizer.json";
          postInstall = ''
            install -Dm644 ${./LICENSE} "$out/share/licenses/beankey/LICENSE"
          '';
          passthru = {
            llamaCpp = pkgs.llama-cpp;
            hunspellEnglish = pkgs.hunspellDicts.en_US;
            hunspellGreek = pkgs.hunspellDicts.el_GR;
          };
          meta = {
            description = "beankey kana-kanji conversion daemon";
            license = pkgs.lib.licenses.mit;
            mainProgram = "beankey-daemon";
            platforms = pkgs.lib.platforms.linux;
          };
        };
      beankeyFcitx5Addon =
        pkgs:
        pkgs.stdenv.mkDerivation {
          pname = "fcitx5-beankey";
          version = "0.1.0";
          src = sourceFor pkgs [
            "LICENSE"
            "fcitx5"
            "proto"
          ];
          cmakeDir = "../fcitx5";
          nativeBuildInputs = [
            pkgs.cmake
            pkgs.ninja
            pkgs.pkg-config
            pkgs.protobuf
          ];
          buildInputs = [
            pkgs.fcitx5
            pkgs.protobuf
          ];
          cmakeFlags = [
            "-DBEANKEY_DAEMON_PATH=${beankeyDaemon pkgs}/bin/beankey-daemon"
            "-DBEANKEY_CONFIG_PATH=/etc/beankey/config.toml"
          ];
          doCheck = true;
          postInstall = ''
            install -Dm644 ${./LICENSE} "$out/share/licenses/beankey/LICENSE"
          '';
          meta = {
            description = "Fcitx5 input method addon for beankey";
            license = pkgs.lib.licenses.mit;
            platforms = pkgs.lib.platforms.linux;
          };
        };
    in
    {
      nixosModules.default = import ./nix/module.nix { inherit self; };

      packages = forAllSystems (
        system:
        let
          pkgs = pkgsFor system;
        in
        {
          dictionary = beankeyDictionary pkgs;
          emoji = beankeyEmoji pkgs;
          model = beankeyModel pkgs;
          tokenizer = beankeyTokenizer pkgs;
        }
        // pkgs.lib.optionalAttrs pkgs.stdenv.hostPlatform.isLinux {
          daemon = beankeyDaemon pkgs;
          fcitx5-addon = beankeyFcitx5Addon pkgs;
        }
      );

      devShells = forAllSystems (
        system:
        let
          pkgs = pkgsFor system;
        in
        {
          default = pkgs.mkShell {
            packages = developmentPackages pkgs;
            buildInputs = [
              pkgs.hunspell
              pkgs.llama-cpp
              pkgs.marisa
            ]
            ++ pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux [ pkgs.fcitx5 ];
            RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";
            BEANKEY_TEST_EN_US_DICTIONARY = "${pkgs.hunspellDicts.en_US}/share/hunspell/en_US";
            BEANKEY_TEST_EL_GR_DICTIONARY = "${pkgs.hunspellDicts.el_GR}/share/hunspell/el_GR";
            BEANKEY_TEST_ZENZ_TOKENIZER = "${beankeyTokenizer pkgs}/share/beankey/tokenizer/tokenizer.json";
          };
        }
      );

      formatter = forAllSystems (system: (pkgsFor system).nixfmt);

      checks = forAllSystems (
        system:
        let
          pkgs = pkgsFor system;
          moduleEvaluation = nixpkgs.lib.nixosSystem {
            inherit system;
            modules = [
              self.nixosModules.default
              {
                programs.beankey.enable = true;
                system.stateVersion = "26.05";
              }
            ];
          };
          moduleConfig = moduleEvaluation.config;
          moduleConfigSource =
            assert builtins.elem self.packages.${system}.fcitx5-addon
              moduleConfig.i18n.inputMethod.fcitx5.addons;
            assert builtins.elem self.packages.${system}.daemon moduleConfig.environment.systemPackages;
            assert !(moduleConfig.systemd.services ? beankey);
            assert !(moduleConfig.systemd.sockets ? beankey);
            moduleConfig.environment.etc."beankey/config.toml".source;
        in
        {
          cargo-metadata =
            pkgs.runCommand "beankey-cargo-metadata"
              {
                nativeBuildInputs = [ pkgs.cargo ];
              }
              ''
                cp ${./Cargo.toml} Cargo.toml
                cargo metadata \
                  --format-version 1 \
                  --no-deps \
                  --manifest-path Cargo.toml \
                  >/dev/null
                touch "$out"
              '';

          development-tools =
            pkgs.runCommand "beankey-development-tools"
              {
                nativeBuildInputs = developmentPackages pkgs;
              }
              ''
                cargo --version
                clang --version
                clang-format --version
                printf '%s\n' \
                  '#include <string_view>' \
                  'static_assert(std::string_view{"beankey"}.size() == 7);' \
                  | clang++ -std=c++20 -x c++ -fsyntax-only -
                cmake --version
                ninja --version
                pkg-config --version
                protoc --version
                rustc --version
                touch "$out"
              '';
        }
        // pkgs.lib.optionalAttrs pkgs.stdenv.hostPlatform.isLinux {
          nixos-module = pkgs.runCommand "beankey-nixos-module" { } ''
            config=${moduleConfigSource}
            grep -F 'dictionary = "${self.packages.${system}.dictionary}/share/beankey/dictionary"' "$config"
            grep -F 'model = "${self.packages.${system}.model}/share/beankey/model/ggml-model-Q5_K_M.gguf"' "$config"
            grep -F 'llama_backend_directory = "${self.packages.${system}.daemon.llamaCpp}/bin"' "$config"
            grep -F 'runtime_socket = "beankey/daemon.sock"' "$config"
            touch "$out"
          '';
        }
      );
    };
}
