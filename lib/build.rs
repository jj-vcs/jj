// Copyright 2020 Google LLC
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
// https://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use std::path::Path;

fn main() {
    let out_dir = format!("{}/protos", std::env::var("OUT_DIR").unwrap());

    if Path::new(&out_dir).exists() {
        std::fs::remove_dir_all(&out_dir).unwrap();
    }
    std::fs::create_dir(&out_dir).unwrap();

    let input = &[
        "src/protos/op_store.proto",
        "src/protos/store.proto",
        "src/protos/working_copy.proto",
    ];
    protobuf_codegen::Codegen::new()
        .pure()
        .out_dir(out_dir)
        .inputs(input)
        .include("src/protos")
        .cargo_out_dir("protos")
        .run_from_script();
    println!("cargo:rerun-if-changed=build.rs");
    for file in input {
        println!("cargo:rerun-if-changed={}", file);
    }

    if let Some(true) = version_check::supports_feature("map_first_last") {
        println!("cargo:rustc-cfg=feature=\"map_first_last\"");
    }
}
