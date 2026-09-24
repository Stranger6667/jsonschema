# frozen_string_literal: true

require "mkmf"
require "rb_sys/mkmf"

create_rust_makefile("jsonschema_bench_magnus") do |r|
  r.auto_install_rust_toolchain = false
end

# rb_sys exports one manifest directory per rake process, so a second extension built from the
# jsonschema-rb Rakefile inherits the first crate's; a plain assignment outranks the environment.
File.write("Makefile", "RB_SYS_CARGO_MANIFEST_DIR = #{__dir__}\n", mode: "a")
