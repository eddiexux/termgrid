# termgrid Homebrew Formula
#
# This formula is intended to be hosted in a separate tap repository:
#   https://github.com/eddiexux/homebrew-tap
#
# To use:
#   brew tap eddiexux/tap
#   brew install termgrid
#
# To publish, copy this file to your homebrew-tap repo at:
#   Formula/termgrid.rb

class Termgrid < Formula
  desc "Terminal multiplexer with Git context awareness"
  homepage "https://github.com/eddiexux/termgrid"
  url "https://github.com/eddiexux/termgrid/archive/refs/tags/v0.1.0.tar.gz"
  license any_of: ["MIT", "Apache-2.0"]

  depends_on "rust" => :build
  depends_on "libgit2"
  depends_on "openssl"

  def install
    system "cargo", "install", *std_cargo_args
  end

  test do
    assert_match "termgrid", shell_output("#{bin}/termgrid --help")
  end
end
