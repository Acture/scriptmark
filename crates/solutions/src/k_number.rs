fn is_k_number<T: Into<usize>>(n: T) -> bool {
    let converted_n = n.into();
    let s = converted_n.to_string();
    let len = s.len();

    for i in 1..len {
        let (left, right) = s.split_at(i);

        match (left.parse::<usize>(), right.parse::<usize>()) {
            (Ok(left_num), Ok(right_num)) => {
                if (left_num + right_num).pow(2) == converted_n {
                    return true;
                }
            }
            _ => continue,
        }
    }

    false
}
fn find_k_number(begin: usize, end: usize) -> Vec<usize> {
    (begin..=end)
        .filter_map(|x| if is_k_number(x) { Some(x) } else { None })
        .collect()
}
