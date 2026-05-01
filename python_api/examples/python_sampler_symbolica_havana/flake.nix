{
  description = "Minimal flake runtime for a Symbolica-backed Havana sampler example";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-24.11";

  outputs = { self, nixpkgs }:
    let
      system = "x86_64-linux";
      pkgs = import nixpkgs { inherit system; };
      hostPython = pkgs.writeShellScriptBin "python" ''
        exec /usr/bin/python "$@"
      '';
    in {
      packages.${system}.runtime = pkgs.symlinkJoin {
        name = "python-sampler-symbolica-havana-runtime";
        paths = [ hostPython ./src ];
      };
    };
}
