# Homebrew formula for zw (Zellij Workbench).
#
#   brew tap LeON-Nie-code/zellij-workbench https://github.com/LeON-Nie-code/zellij-workbench
#   brew install LeON-Nie-code/zellij-workbench/zw
#
# Placeholder checksums/URLs: fill these in once a tagged release exists.
class Zw < Formula
  desc "Terminal workspace memory manager for local and remote zellij sessions"
  homepage "https://github.com/LeON-Nie-code/zellij-workbench"
  url "https://github.com/LeON-Nie-code/zellij-workbench/archive/refs/tags/v0.1.0.tar.gz"
  sha256 ""
  license "MIT"
  head "https://github.com/LeON-Nie-code/zellij-workbench.git", branch: "main"

  depends_on "rust" => :build
  depends_on "zellij"

  def install
    system "cargo", "install", *std_cargo_args(path: ".")
  end

  test do
    assert_match "zw", shell_output("#{bin}/zw --help")
  end
end
