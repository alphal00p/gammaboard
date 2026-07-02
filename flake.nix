{
  description = "Gammaboard development shell";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";

    crane = {
      url = "github:ipetkov/crane";
      # inputs.nixpkgs.follows = "nixpkgs";
    };

    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.rust-analyzer-src.follows = "";
    };

    flake-utils.url = "github:numtide/flake-utils";

  };

  outputs = {
    self,
    nixpkgs,
    crane,
    fenix,
    flake-utils,
    ...
  }:
    flake-utils.lib.eachDefaultSystem (system: let
      pkgs = nixpkgs.legacyPackages.${system};
      inherit (pkgs) lib;

      craneLib =
        (crane.mkLib nixpkgs.legacyPackages.${system}).overrideToolchain
        fenix.packages.${system}.stable.toolchain;

      # Host Rust target triple, e.g. x86_64-unknown-linux-gnu
      rustTarget = pkgs.stdenv.hostPlatform.rust.rustcTarget;

      # Env var name Cargo uses to pick the linker for this target
      cargoLinkerVar = "CARGO_TARGET_${lib.toUpper (lib.replaceStrings ["-"] ["_"] rustTarget)}_LINKER";

      # Force the "Nix cc wrapper" as both C compiler and Rust linker.
      nixCc = "${pkgs.stdenv.cc}/bin/cc";
      nixCxx = "${pkgs.stdenv.cc}/bin/c++";

      # Runtime library search path for locally-built binaries.
      runtimeLibPath = lib.makeLibraryPath [
        pkgs.python313
        pkgs.gmp
        pkgs.mpfr
        pkgs.libmpc
        pkgs.openssl
        pkgs.stdenv.cc.cc.lib
        pkgs.zlib
        "/run/opengl-driver"
      ];

      # Common arguments can be set here to avoid repeating them later
    in {
      devShells.default = craneLib.devShell {

        RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";
        GLIBC_TUNABLES = "glibc.rtld.optional_static_tls=10000";

        # Mirror the same hard overrides in the interactive shell.
        CC = nixCc;
        CXX = nixCxx;
        "${cargoLinkerVar}" = nixCc;
        RUSTFLAGS = "-C linker=${nixCc}";

        # MG7-generated makefiles hardcode /bin/bash, which does not exist on
        # non-FHS Nix systems. A make command-line variable overrides that value.
        MAKEFLAGS = "SHELL=${pkgs.bash}/bin/bash";

        # Make libpython + libgmp/libmpfr/libmpc visible to local binaries.
        LD_LIBRARY_PATH = runtimeLibPath;
        DYLD_LIBRARY_PATH = runtimeLibPath;

        packages = with pkgs; [
          just
          openssl
          gmp
          mpfr
          libmpc
          # MadGraph7/MadSpace source build and runtime dependencies.
          cmake
          ninja
          gnumake
          git
          curl
          wget
          cacert
          gnutar
          xz
          pkg-config
          patchelf
          openblas
          lhapdf
          python313
          python313Packages.pip
          python313Packages.setuptools
          python313Packages.wheel
          python313Packages.packaging
          python313Packages.scikit-build-core
          python313Packages.numpy
          cargo-nextest
          cargo-watch
          gfortran
          gcc
          uv
          rust-analyzer
          virtualenv
          postgresql
          nginx
          apptainer
          sqlx-cli
          nodejs
          python313Packages.six
          (pkgs.rustPlatform.buildRustPackage rec {
            pname = "clinnet";
            version = "0.1.8";
            src = pkgs.fetchCrate {
              inherit pname version;
              sha256 = "sha256-CbZBHbf+8bIkdiSI5LMFO2Qc3zDr9UEBEry+fZOuep8=";
            };
            cargoHash = "sha256-GTixU2ZJZVMrEWLOfWjEnXMVLG2+cpkPbJuNnkTuFfo=";
          })
        ];
      };
    });
}
