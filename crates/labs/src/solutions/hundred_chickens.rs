fn buy_hundred_chickens() -> Vec<(i32, i32, i32)> {
    (0..=20)
        .flat_map(|x| {
            (0..=33).filter_map(move |y| {
                let z = 100 - x - y;
                if z % 3 == 0 && 5 * x + 3 * y + z / 3 == 100 {
                    Some((x, y, z))
                } else {
                    None
                }
            })
        })
        .collect()
}
