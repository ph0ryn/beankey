{
  description = "Development environment for beankey";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    self.submodules = true;
  };

  outputs =
    { nixpkgs, ... }:
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
      beankeyModel =
        pkgs:
        pkgs.fetchurl {
          url = "https://huggingface.co/Miwa-Keita/zenz-v3.2-small-gguf/resolve/c67e03e07d215c869f591b274c1631170d3e11fe/ggml-model-Q5_K_M.gguf";
          hash = "sha256-KcIj1MIzJ7gP0T67WrJVUFekYxeZfV2jkVhP++8NtnM=";
        };
    in
    {
      packages = forAllSystems (system: {
        model = beankeyModel (pkgsFor system);
      });

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
            ]
            ++ pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux [ pkgs.fcitx5 ];
            RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";
            BEANKEY_TEST_EN_US_DICTIONARY = "${pkgs.hunspellDicts.en_US}/share/hunspell/en_US";
            BEANKEY_TEST_EL_GR_DICTIONARY = "${pkgs.hunspellDicts.el_GR}/share/hunspell/el_GR";
          };
        }
      );

      formatter = forAllSystems (system: (pkgsFor system).nixfmt);

      checks = forAllSystems (
        system:
        let
          pkgs = pkgsFor system;
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
                  | clang++ -std=c++17 -x c++ -fsyntax-only -
                cmake --version
                ninja --version
                pkg-config --version
                protoc --version
                rustc --version
                touch "$out"
              '';
        }
      );
    };
}
