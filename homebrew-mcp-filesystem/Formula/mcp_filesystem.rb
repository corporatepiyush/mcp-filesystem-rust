class McpFilesystem < Formula
  desc "High-performance MCP server for filesystem access"
  homepage "https://github.com/corporatepiyush/mcp-filesystem-rust"
  url "https://github.com/corporatepiyush/mcp-filesystem-rust/archive/refs/tags/v1.0.0.tar.gz"
  sha256 "3364ae96f5080eaf526146b6216fc3de95c1af0ad4a5613c67592cd63960637a"
  license "Apache-2.0"

  depends_on "rust" => :build

  def install
    system "cargo", "install", *std_cargo_args
  end

  test do
    assert_predicate bin/"mcp-filesystem", :exist?
  end
end
