use nom::*;

use std::str;

use crate::internals::{Hash, HoconInternal, HoconValue};

named!(
    space<()>,
    map!(
        many0!(alt!(
            tag!(" ")
                | tag!("\t")
                | tag!("\u{feff}")
                | tag!("\u{00a0}")
                | tag!("\u{2007}")
                | tag!("\u{202f}")
        )),
        |_| ()
    )
);

macro_rules! sp (
    ($i:expr, $($args:tt)*) => (
        {
            use nom::Convert;
            use nom::Err;

            match sep!($i, space, $($args)*) {
                Err(e)         => Err(e),
                Ok((i1, o))    => {
                    match space(i1) {
                        Err(e) => Err(Err::convert(e)),
                        Ok((i2, _))    => Ok((i2, o))
                    }
                }
            }
        }
    )
);

named!(possible_comment<Option<()>>, opt!(multiline_comment));
named!(
    multiline_comment<()>,
    do_parse!(many0!(newline) >> comment >> many0!(alt!(newline => { |_| () } | comment)) >> ())
);
named!(
    comment<()>,
    sp!(do_parse!(
        alt!(tag!("//") | tag!("#")) >> take_until_and_consume!("\n") >> ()
    ))
);

named!(integer<i64>, flat_map!(recognize_float, parse_to!(i64)));

named!(float<f64>, flat_map!(recognize_float, parse_to!(f64)));

named!(null, tag!("null"));

//FIXME: verify how json strings are formatted
named!(
    string<&str>,
    delimited!(
        char!('"'),
        map_res!(
            escaped!(none_of!("\"\n"), '\\', one_of!("\"n\\")),
            str::from_utf8
        ),
        char!('"')
    )
);

named!(
    boolean<bool>,
    alt!(value!(false, tag!("false")) | value!(true, tag!("true")))
);

macro_rules! take_until_tag1 (
    ($input:expr, $arr:expr) => (
        {
            use nom::lib::std::result::Result::*;
            use nom::lib::std::option::Option::*;
            use nom::{Err,Needed,IResult,need_more_err,ErrorKind};

            use nom::InputIter;
            use nom::InputTake;

            let res: IResult<_, _> = match $input.iter_indices()
                .fold((0, None),
                    |(old_c, pos), (i, c)| {
                        if pos.is_some() {
                            (old_c, pos)
                        } else if $arr.contains(&str::from_utf8(&[c]).unwrap()) {
                            (c, Some(i))
                        } else {
                            if $arr.contains(&str::from_utf8(&[old_c, c]).unwrap()) {
                                (c, Some(i - 1))
                            } else {
                                (c, None)
                            }
                        }
                    }
                )
            {
                (_, Some(0)) => Err(Err::Error(error_position!($input, ErrorKind::TakeUntilEither::<u32>))),
                (_, Some(n)) => Ok($input.take_split(n)),
                (_, None)    => need_more_err($input, Needed::Size(1), ErrorKind::TakeUntilEither::<u32>)
            };
            res
        }
    );
);

named!(
    unquoted_string<&str>,
    map_res!(
        complete!(take_until_tag1!([
            "$", "\"", "{", "}", "[", "]", ":", "=", ",", "+", "#", "`", "^", "?", "!", "@", "*",
            "&", "'", "\\", "\t", "\n", "//"
        ])),
        |v| str::from_utf8(v).map(str::trim)
    )
);

named!(
    path_substitution<HoconValue>,
    delimited!(alt!(tag!("${?") | tag!("${")), value, char!('}'))
);

named_args!(
    array<'a>(file_root: Option<&'a str>, depth: usize)<Vec<HoconInternal>>,
    sp!(delimited!(
        do_parse!(char!('[') >> many0!(newline) >> ()),
        separated_list!(separators, call!(wrapper, file_root, depth)),
        call!(closing, ']')
    ))
);

named_args!(
    key_value<'a>(file_root: Option<&'a str>, depth: usize)<Hash>,
    do_parse!(
        ws!(possible_comment)
            >> pair: sp!(alt!(
                call!(include) => { |path| HoconInternal::from_include(file_root, path, depth).internal } |
                separated_pair!(ws!(string), ws!(alt!(char!(':') | char!('='))), call!(wrapper, file_root, depth))
                    => { |(s, h): (&str, HoconInternal)|
                        HoconInternal::from_object(h.internal)
                            .add_to_path(vec![HoconValue::String(String::from(s))]).internal
                    } |
                pair!(ws!(string), call!(hash, file_root, depth))
                    => { |(s, h): (&str, Hash)|
                        HoconInternal::from_object(h)
                            .add_to_path(vec![HoconValue::String(String::from(s))]).internal
                    } |
                separated_pair!(ws!(unquoted_string), ws!(alt!(char!(':') | char!('='))), call!(wrapper, file_root, depth))
                    => { |(s, h): (&str, HoconInternal)|
                        HoconInternal::from_object(h.internal)
                            .add_to_path(vec![HoconValue::UnquotedString(String::from(s))]).internal
                    } |
                pair!(ws!(unquoted_string), call!(hash, file_root, depth))
                    => { |(s, h): (&str, Hash)|
                        HoconInternal::from_object(h)
                            .add_to_path(vec![HoconValue::UnquotedString(String::from(s))]).internal
                    }
            ))
            >> (pair)
    )
);

named!(
    separators<()>,
    alt!(
        sp!(multiline_comment) => { |_| () } |
        sp!(many1!(newline)) => { |_| () } |
        ws!(do_parse!(char!(',') >> possible_comment>> ())) => { |_| () }
    )
);

named_args!(
    separated_hashlist<'a>(file_root: Option<&'a str>, depth: usize)<Vec<Hash>>,
    separated_list!(separators, call!(key_value, file_root, depth))
);

named_args!(
    closing(closing_char: char)<()>,
    do_parse!(opt!(separators) >> char!(closing_char) >> ())
);

named_args!(
    hash<'a>(file_root: Option<&'a str>, depth: usize)<Hash>,
    sp!(map!(
        delimited!(char!('{'), call!(separated_hashlist, file_root, depth), call!(closing, '}')),
        |tuple_vec| tuple_vec.into_iter().flat_map(|h| h.into_iter()).collect()
    ))
);

named_args!(
    root_hash<'a>(file_root: Option<&'a str>, depth: usize)<Hash>,
    sp!(map!(
        do_parse!(not!(char!('{')) >> list: call!(separated_hashlist, file_root, depth) >> (list)),
        |tuple_vec| tuple_vec.into_iter().flat_map(|h| h.into_iter()).collect()
    ))
);

named!(
    single_value<HoconValue>,
    sp!(alt!(
        string  =>           { |s| HoconValue::String(String::from(s))           } |
        integer =>           { |i| HoconValue::Integer(i)                        } |
        float   =>           { |f| HoconValue::Real(f)                           } |
        boolean =>           { |b| HoconValue::Boolean(b)                        } |
        null =>              { |_| HoconValue::Null                              } |
        path_substitution => { |p| HoconValue::PathSubstitution(Box::new(p))     } |
        unquoted_string =>   { |s| HoconValue::UnquotedString(String::from(s))   }
    ))
);

named!(
    value<HoconValue>,
    map!(
        do_parse!(
            possible_comment
                >> first_value: single_value
                >> remaining_values: many0!(sp!(single_value))
                >> (first_value, remaining_values)
        ),
        |(first_value, mut remaining_values)| if remaining_values.is_empty() {
            first_value
        } else {
            let mut values = vec![first_value];
            values.append(&mut remaining_values);
            HoconValue::Concat(values)
        }
    )
);

named!(
    include<&str>,
    do_parse!(
        tag!("include ")
            >> file_name:
                sp!(alt!(
                    call!(string)
                        | do_parse!(tag!("file(") >> file_name: string >> tag!(")") >> (file_name))
                ))
            >> (file_name)
    )
);

named_args!(
    root_include<'a>(file_root: Option<&'a str>, depth: usize)<HoconInternal>,
    map!(
        do_parse!(file_name: ws!(include) >> doc: call!(root, file_root, depth) >> ((file_name, doc))),
        |(file_name, mut doc)| match file_root {
            Some(root) => doc.add_include(&root, file_name, depth),
            None => doc,
        }
    )
);

named_args!(
    wrapper<'a>(file_root: Option<&'a str>, depth: usize)<HoconInternal>,
    do_parse!(
        possible_comment
            >> wrapped:
                alt!(
                    call!(hash, file_root, depth)  => { |h| HoconInternal::from_object(h)                    } |
                    call!(array, file_root, depth) => { |a| HoconInternal::from_array(a)                     } |
                    include                        => { |f| HoconInternal::from_include(file_root, f, depth) } |
                    value                          => { |v| HoconInternal::from_value(v)                     }
                )
            >> (wrapped)
    )
);

named_args!(
    pub(crate) root<'a>(file_root: Option<&'a str>, depth: usize)<HoconInternal>,
    do_parse!(
        possible_comment
            >> wrapped:
                alt!(
                    call!(root_include, file_root, depth) => { |d| d                             } |
                    call!(root_hash, file_root, depth)    => { |h| HoconInternal::from_object(h) } |
                    call!(hash, file_root, depth)         => { |h| HoconInternal::from_object(h) } |
                    call!(array, file_root, depth)        => { |a| HoconInternal::from_array(a)  } |
                    value                                 => { |v| HoconInternal::from_value(v)  }
                )
            >> (wrapped)
    )
);
