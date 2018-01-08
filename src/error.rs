// Copyright (c) 2017 repomons developers
//
// Licensed under the Apache License, Version 2.0
// <LICENSE-APACHE or http://www.apache.org/licenses/LICENSE-2.0> or the MIT
// license <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. All files in the project carrying such notice may not be copied,
// modified, or distributed except according to those terms.

//! `repomon` errors
error_chain!{
    foreign_links {
        AddrParse(::std::net::AddrParseError);
        Git2(::git2::Error);
        Infallible(::std::convert::Infallible);
        Io(::std::io::Error);
        Repomon(::repomon::Error);
    }
}
