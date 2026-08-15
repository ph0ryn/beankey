{
  developmentPackages,
  nixpkgs,
  pkgs,
  self,
  system,
}:

let
  moduleEvaluation = nixpkgs.lib.nixosSystem {
    inherit system;
    modules = [
      self.nixosModules.default
      {
        programs.beankey = {
          enable = true;
          conversion = {
            typeBackslash = true;
            typeHalfSpace = true;
            optionDirectFullWidthInput = true;
            punctuationStyle = "period_and_comma";
          };
        };
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
        cp ${../Cargo.toml} Cargo.toml
        cp -R ${../crates} crates
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
        nativeBuildInputs = developmentPackages;
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
    grep -F 'keyboard_language = "japanese"' "$config"
    grep -F 'japanese_prediction = "disabled"' "$config"
    grep -F 'foreign_prediction = "disabled"' "$config"
    grep -F 'full_width_roman = true' "$config"
    grep -F 'half_width_kana = false' "$config"
    grep -F 'live_conversion = true' "$config"
    grep -F 'type_backslash = true' "$config"
    grep -F 'type_half_space = true' "$config"
    grep -F 'option_direct_full_width_input = true' "$config"
    grep -F 'punctuation_style = "period_and_comma"' "$config"
    grep -F 'mode = "input_and_output"' "$config"
    grep -F 'max_count = 65536' "$config"
    grep -F 'inference_limit = 5' "$config"
    grep -F 'enable_alignment_separator = true' "$config"
    touch "$out"
  '';
}
