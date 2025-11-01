#![allow(unused_macros)]
macro_rules! str_tuple2 {
    ($app:ident, $value:ident, $method:ident) => {{
        if let Some(vec) = $value.as_vec() {
            for ys in vec {
                if let Some(tup) = ys.as_vec() {
                    debug_assert_eq!(2, tup.len());
                    $app = $app.$method(str_str!(tup[0]), str_str!(tup[1]));
                } else {
                    panic!("Failed to convert YAML value to vec");
                }
            }
        } else {
            panic!("Failed to convert YAML value to vec");
        }
        $app
    }};
}

macro_rules! str_tuple3 {
    ($app:ident, $value:ident, $method:ident) => {{
        if let Some(vec) = $value.as_vec() {
            for ys in vec {
                if let Some(tup) = ys.as_vec() {
                    debug_assert_eq!(3, tup.len());
                    $app = $app.$method(str_str!(tup[0]), str_opt_str!(tup[1]), str_opt_str!(tup[2]));
                } else {
                    panic!("Failed to convert YAML value to vec");
                }
            }
        } else {
            panic!("Failed to convert YAML value to vec");
        }
        $app
    }};
}

macro_rules! str_vec_or_str {
    ($app:ident, $value:ident, $method:ident) => {{
        let maybe_vec = $value.as_vec();
        if let Some(vec) = maybe_vec {
            for ys in vec {
                if let Some(s) = ys.as_str() {
                    $app = $app.$method(s);
                } else {
                    panic!("Failed to convert YAML value {:?} to a string", ys);
                }
            }
        } else {
            if let Some(s) = $value.as_str() {
                $app = $app.$method(s);
            } else {
                panic!("Failed to convert YAML value {:?} to either a vec or string", $value);
            }
        }
        $app
    }};
}

macro_rules! str_opt_str {
    ($value:ident) => {{ if let Some(s) = $value.as_str() { Some(s) } else { None } }};
}

macro_rules! str_str {
    ($value:ident) => {{
        if let Some(s) = $value.as_str() {
            s
        } else {
            panic!("Failed to convert YAML value {:?} to a string", $value);
        }
    }};
}

macro_rules! str_bool {
    ($value:ident) => {{
        if let Some(b) = $value.as_bool() {
            b
        } else {
            panic!("Failed to convert YAML value {:?} to a boolean", $value);
        }
    }};
}

macro_rules! str_opt_bool {
    ($value:ident) => {{ if let Some(b) = $value.as_bool() { Some(b) } else { None } }};
}

macro_rules! str_i64 {
    ($value:ident) => {{
        if let Some(i) = $value.as_i64() {
            i
        } else {
            panic!("Failed to convert YAML value {:?} to an i64", $value);
        }
    }};
}

macro_rules! str_opt_i64 {
    ($value:ident) => {{ if let Some(i) = $value.as_i64() { Some(i) } else { None } }};
}

macro_rules! str_u64 {
    ($value:ident) => {{
        if let Some(u) = $value.as_u64() {
            u
        } else {
            panic!("Failed to convert YAML value {:?} to a u64", $value);
        }
    }};
}

macro_rules! str_opt_u64 {
    ($value:ident) => {{ if let Some(u) = $value.as_u64() { Some(u) } else { None } }};
}

macro_rules! str_f64 {
    ($value:ident) => {{
        if let Some(f) = $value.as_f64() {
            f
        } else {
            panic!("Failed to convert YAML value {:?} to an f64", $value);
        }
    }};
}

macro_rules! str_opt_f64 {
    ($value:ident) => {{ if let Some(f) = $value.as_f64() { Some(f) } else { None } }};
}

macro_rules! str_vec {
    ($value:ident) => {{
        if let Some(vec) = $value.as_vec() {
            vec.iter().map(|y| str_str!(y)).collect::<Vec<_>>()
        } else {
            panic!("Failed to convert YAML value {:?} to a vec", $value);
        }
    }};
}

macro_rules! str_opt_vec {
    ($value:ident) => {{
        if let Some(vec) = $value.as_vec() {
            Some(vec.iter().map(|y| str_str!(y)).collect::<Vec<_>>())
        } else {
            None
        }
    }};
}
