class McpFilesystem < Formula
  desc "High-performance MCP server for filesystem access"
  homepage "https://github.com/corporatepiyush/mcp-filesystem-rust"
  url "https://github.com/corporatepiyush/mcp-filesystem-rust/archive/refs/tags/v1.4.0.tar.gz"
  sha256 "e79124901b96ae32bf3b95f3e9f5f4f7834fe46eba076cb6a5f49888754d67a8"
  license "Apache-2.0"

  depends_on "rust" => :build

  def install
    system "cargo", "install", *std_cargo_args
  end

  test do
    assert_predicate bin/"mcp-filesystem", :exist?
  end
end
