# frozen_string_literal: true

require "mkmf"
require "rb_sys/mkmf"

create_rust_makefile("jsonschema/jsonschema_rb") do |r|
  r.auto_install_rust_toolchain = false

  musl_target =
    ENV["CARGO_BUILD_TARGET"]&.include?("musl") ||
    File.exist?("/etc/alpine-release") ||
    begin
      `ldd --version 2>&1` =~ /musl/
    rescue StandardError
      false
    end ||
    `rustc -vV 2>/dev/null`[/host: (.+)/, 1]&.include?("musl")

  if musl_target
    # Disable static CRT on musl.
    r.extra_rustflags = ["-C", "target-feature=-crt-static"]
  end
end
