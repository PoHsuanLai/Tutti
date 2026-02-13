/// Maps logical speaker positions to output buffer channel indices.
#[derive(Debug, Clone, Copy)]
pub struct ChannelLayout {
    pub left: usize,
    pub right: usize,
    pub center: Option<usize>,
    pub lfe: Option<usize>,
    pub surround_left: Option<usize>,
    pub surround_right: Option<usize>,
    pub rear_left: Option<usize>,
    pub rear_right: Option<usize>,
    pub height_front_left: Option<usize>,
    pub height_front_right: Option<usize>,
    pub height_rear_left: Option<usize>,
    pub height_rear_right: Option<usize>,
}

impl ChannelLayout {
    pub fn stereo() -> Self {
        Self {
            left: 0,
            right: 1,
            center: None,
            lfe: None,
            surround_left: None,
            surround_right: None,
            rear_left: None,
            rear_right: None,
            height_front_left: None,
            height_front_right: None,
            height_rear_left: None,
            height_rear_right: None,
        }
    }

    pub fn surround_5_1() -> Self {
        Self {
            left: 0,
            right: 1,
            center: Some(2),
            lfe: Some(3),
            surround_left: Some(4),
            surround_right: Some(5),
            rear_left: None,
            rear_right: None,
            height_front_left: None,
            height_front_right: None,
            height_rear_left: None,
            height_rear_right: None,
        }
    }

    pub fn surround_7_1() -> Self {
        Self {
            left: 0,
            right: 1,
            center: Some(2),
            lfe: Some(3),
            surround_left: Some(4),
            surround_right: Some(5),
            rear_left: Some(6),
            rear_right: Some(7),
            height_front_left: None,
            height_front_right: None,
            height_rear_left: None,
            height_rear_right: None,
        }
    }

    pub fn atmos_7_1_4() -> Self {
        Self {
            left: 0,
            right: 1,
            center: Some(2),
            lfe: Some(3),
            surround_left: Some(4),
            surround_right: Some(5),
            rear_left: Some(6),
            rear_right: Some(7),
            height_front_left: Some(8),
            height_front_right: Some(9),
            height_rear_left: Some(10),
            height_rear_right: Some(11),
        }
    }

    pub fn num_channels(&self) -> usize {
        let mut max = core::cmp::max(self.right, self.left);
        if let Some(c) = self.center {
            max = core::cmp::max(max, c);
        }
        if let Some(c) = self.lfe {
            max = core::cmp::max(max, c);
        }
        if let Some(c) = self.surround_left {
            max = core::cmp::max(max, c);
        }
        if let Some(c) = self.surround_right {
            max = core::cmp::max(max, c);
        }
        if let Some(c) = self.rear_left {
            max = core::cmp::max(max, c);
        }
        if let Some(c) = self.rear_right {
            max = core::cmp::max(max, c);
        }
        if let Some(c) = self.height_front_left {
            max = core::cmp::max(max, c);
        }
        if let Some(c) = self.height_front_right {
            max = core::cmp::max(max, c);
        }
        if let Some(c) = self.height_rear_left {
            max = core::cmp::max(max, c);
        }
        if let Some(c) = self.height_rear_right {
            max = core::cmp::max(max, c);
        }
        max + 1
    }
}
