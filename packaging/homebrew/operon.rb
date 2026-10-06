# Homebrew formula for Operon, built from source against a release tag.
# Documentation: https://docs.brew.sh/Formula-Cookbook
#
# Release flow: tag vX.Y.Z, run scripts/release.sh, then paste the archive
# hash from dist/SHA256SUMS into the sha256 stanza below. If the GitHub tag
# tarball and the git archive ever differ, hash the exact URL below instead:
#   curl -fsSL <url> | shasum -a 256
class Operon < Formula
  desc "Gene-expression programming language: Total Grammar toolchain, Rust core, C++ kernels"
  homepage "https://github.com/WasewaseX/operon-lang-dev"
  url "https://github.com/WasewaseX/operon-lang-dev/archive/refs/tags/v2.9.0.tar.gz"
  # TODO(release): replace the all-zero placeholder with the sha256 printed by
  # scripts/release.sh (the dist/SHA256SUMS line for this version) once the
  # v2.9.0 tag exists. brew audit --strict fails on the placeholder by design,
  # so a formula with an unfilled digest cannot ship unnoticed.
  sha256 "0000000000000000000000000000000000000000000000000000000000000000"
  license "MIT"

  depends_on "rust" => :build
  # C/C++ performance kernels: runtime/codon_kernel.cpp is compiled by
  # build.rs (cc crate) and scripts/build.sh (g++) during the release build.
  depends_on "gcc"

  def install
    # build.sh: cargo build --release, g++ kernel objects, copies to bin/
    system "./scripts/build.sh"
    bin.install "bin/operon"
    bin.install "bin/operon-ls"

    # Runtime support files live in libexec; bin/std is the exe-relative
    # symlink the module loader resolves next to the binary (src/genes.rs:
    # exe-dir/std and exe-dir/../std candidates, canonicalized by the
    # capability layer). bootstrap/ (Python oracle tooling) rides along for
    # users who want to run the differential harness.
    libexec.install "std", "bootstrap"
    (bin/"std").install_symlink libexec/"std"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/operon version")
    (testpath/"hello.op").write <<~OP
      gene main() {
          promote("brew test")
      }
    OP
    assert_equal "brew test", shell_output("#{bin}/operon run #{testpath}/hello.op").strip
  end
end
