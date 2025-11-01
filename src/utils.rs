use std::fmt::Debug;

#[allow(dead_code)]
fn print_type_of<T>(t: &T)
where
    T: ?Sized + Debug,
{
    println!("type={} value={:#?}", std::any::type_name::<T>(), t);
}

#[allow(dead_code)]
fn format_items(items: &[&str], before: Option<&str>, between: Option<&str>, after: Option<&str>) -> String {
    let before_str = before.unwrap_or("");
    let between_str = between.unwrap_or("");
    let after_str = after.unwrap_or("");

    if items.is_empty() {
        return format!("{before_str}{after_str}");
    }

    let mut result = String::new();
    result.push_str(before_str);

    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            result.push_str(between_str);
        }
        result.push_str(item);
    }

    result.push_str(after_str);
    result
}
