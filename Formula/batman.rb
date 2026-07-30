class Batman < Formula
  desc "Borderline Awesome Tool for Multiagent Automation by Nikolas"
  homepage "https://github.com/can1357/batman"
  license "MIT"
  version "0.1.0"

  if OS.mac?
    if Hardware::CPU.arm?
      url "https://github.com/can1357/batman/releases/download/v#{version}/batcave-darwin-arm64"
      sha256 "REPLACE_WITH_ACTUAL_SHA256_FOR_DARWIN_ARM64"
    else
      url "https://github.com/can1357/batman/releases/download/v#{version}/batcave-darwin-x64"
      sha256 "REPLACE_WITH_ACTUAL_SHA256_FOR_DARWIN_X64"
    end
  elsif OS.linux?
    if Hardware::CPU.arch == :arm64
      url "https://github.com/can1357/batman/releases/download/v#{version}/batcave-linux-arm64-gnu"
      sha256 "REPLACE_WITH_ACTUAL_SHA256_FOR_LINUX_ARM64"
    else
      url "https://github.com/can1357/batman/releases/download/v#{version}/batcave-linux-x64-gnu"
      sha256 "REPLACE_WITH_ACTUAL_SHA256_FOR_LINUX_X64"
    end
  end

  def install
    bin.install "batcave"
  end

  test do
    system "#{bin}/batcave", "--version"
  end
end
