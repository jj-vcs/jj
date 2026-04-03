// Copyright 2026 The Jujutsu Authors
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

use std::io::Read;

use clap::Parser;

#[derive(Parser, Debug)]
struct Args {
    /// Change the input bytes to upper casses based on the ASCII encoding.
    #[arg(long, default_value_t = false)]
    uppercase: bool,

    /// Abort right after the process starts, particularly before reading
    /// anything from the stdin.
    #[arg(long, default_value_t = false)]
    abort_on_start: bool,

    /// Abort right before the process exits, particularly after writing
    /// everything to the stdout.
    #[arg(long, default_value_t = false)]
    abort_on_end: bool,
}

impl Args {
    fn convert(&self, input: impl Read) -> impl Read {
        TransformedReader {
            inner: input,
            mapper: |buf| {
                let new_contents = if self.uppercase {
                    buf.to_ascii_uppercase()
                } else {
                    buf.to_owned()
                };
                buf.copy_from_slice(&new_contents);
                Ok(())
            },
        }
    }
}

struct TransformedReader<T, F>
where
    T: std::io::Read,
    F: FnMut(&mut [u8]) -> std::io::Result<()>,
{
    inner: T,
    mapper: F,
}

impl<T, F> Read for TransformedReader<T, F>
where
    T: std::io::Read,
    F: FnMut(&mut [u8]) -> std::io::Result<()>,
{
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        (self.mapper)(&mut buf[..n])?;
        Ok(n)
    }
}

fn main() {
    let args = Args::parse();
    if args.abort_on_start {
        panic!("User requested abort on start.");
    }
    let mut output = args.convert(std::io::stdin());
    let mut stdout = std::io::stdout();
    std::io::copy(&mut output, &mut stdout).expect("Failed to write the results to stdout.");
    if args.abort_on_end {
        panic!("User requested abort on end.");
    }
}
