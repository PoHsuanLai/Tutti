//! Graph building macros for TuttiNet.
//!
//! These macros provide ergonomic ways to build audio graphs:
//! - `chain!` - Connect nodes in series
//! - `mix!` - Mix multiple signals together
//! - `split!` - Fan out a signal to multiple destinations
//! - `params!` - Create parameter maps for node instantiation

/// Chain multiple nodes together in a linear signal flow.
///
/// Use `=> output` to pipe the last node to output.
///
/// # Example
/// ```ignore
/// chain!(net, sine_id, filter_id, gain_id, reverb_id => output);
/// let last = chain!(net, sine_id, filter_id); // Returns filter_id
/// ```
#[macro_export]
macro_rules! chain {
    // Chain with => output at the end
    ($net:expr, $first:expr, $second:expr => output) => {{
        $net.pipe($first, $second);
        $net.pipe_output($second);
    }};

    ($net:expr, $first:expr, $second:expr, $($rest:expr),+ => output) => {{
        $net.pipe($first, $second);
        chain!($net, $second, $($rest),+ => output);
    }};

    // Chain without output (returns last node)
    ($net:expr, $first:expr, $second:expr) => {{
        $net.pipe($first, $second);
        $second
    }};

    ($net:expr, $first:expr, $second:expr, $($rest:expr),+) => {{
        $net.pipe($first, $second);
        chain!($net, $second, $($rest),+)
    }};
}

/// Mix multiple signals into a single node using fundsp's join.
///
/// Supports 2-8 sources (FunDSP uses compile-time sized types).
///
/// # Example
/// ```ignore
/// let mixed = mix!(net, osc1, osc2, osc3);
/// ```
#[macro_export]
macro_rules! mix {
    ($net:expr, $s1:expr, $s2:expr) => {{
        use $crate::dsp::*;
        let m = $net.add(join::<typenum::U2>()).id();
        $net.connect_ports($s1, 0, m, 0);
        $net.connect_ports($s2, 0, m, 1);
        m
    }};
    ($net:expr, $s1:expr, $s2:expr, $s3:expr) => {{
        use $crate::dsp::*;
        let m = $net.add(join::<typenum::U3>()).id();
        $net.connect_ports($s1, 0, m, 0);
        $net.connect_ports($s2, 0, m, 1);
        $net.connect_ports($s3, 0, m, 2);
        m
    }};
    ($net:expr, $s1:expr, $s2:expr, $s3:expr, $s4:expr) => {{
        use $crate::dsp::*;
        let m = $net.add(join::<typenum::U4>()).id();
        $net.connect_ports($s1, 0, m, 0);
        $net.connect_ports($s2, 0, m, 1);
        $net.connect_ports($s3, 0, m, 2);
        $net.connect_ports($s4, 0, m, 3);
        m
    }};
    ($net:expr, $s1:expr, $s2:expr, $s3:expr, $s4:expr, $s5:expr) => {{
        use $crate::dsp::*;
        let m = $net.add(join::<typenum::U5>()).id();
        $net.connect_ports($s1, 0, m, 0);
        $net.connect_ports($s2, 0, m, 1);
        $net.connect_ports($s3, 0, m, 2);
        $net.connect_ports($s4, 0, m, 3);
        $net.connect_ports($s5, 0, m, 4);
        m
    }};
    ($net:expr, $s1:expr, $s2:expr, $s3:expr, $s4:expr, $s5:expr, $s6:expr) => {{
        use $crate::dsp::*;
        let m = $net.add(join::<typenum::U6>()).id();
        $net.connect_ports($s1, 0, m, 0);
        $net.connect_ports($s2, 0, m, 1);
        $net.connect_ports($s3, 0, m, 2);
        $net.connect_ports($s4, 0, m, 3);
        $net.connect_ports($s5, 0, m, 4);
        $net.connect_ports($s6, 0, m, 5);
        m
    }};
    ($net:expr, $s1:expr, $s2:expr, $s3:expr, $s4:expr, $s5:expr, $s6:expr, $s7:expr) => {{
        use $crate::dsp::*;
        let m = $net.add(join::<typenum::U7>()).id();
        $net.connect_ports($s1, 0, m, 0);
        $net.connect_ports($s2, 0, m, 1);
        $net.connect_ports($s3, 0, m, 2);
        $net.connect_ports($s4, 0, m, 3);
        $net.connect_ports($s5, 0, m, 4);
        $net.connect_ports($s6, 0, m, 5);
        $net.connect_ports($s7, 0, m, 6);
        m
    }};
    ($net:expr, $s1:expr, $s2:expr, $s3:expr, $s4:expr, $s5:expr, $s6:expr, $s7:expr, $s8:expr) => {{
        use $crate::dsp::*;
        let m = $net.add(join::<typenum::U8>()).id();
        $net.connect_ports($s1, 0, m, 0);
        $net.connect_ports($s2, 0, m, 1);
        $net.connect_ports($s3, 0, m, 2);
        $net.connect_ports($s4, 0, m, 3);
        $net.connect_ports($s5, 0, m, 4);
        $net.connect_ports($s6, 0, m, 5);
        $net.connect_ports($s7, 0, m, 6);
        $net.connect_ports($s8, 0, m, 7);
        m
    }};
}

/// Split a signal to multiple destinations (fan-out).
///
/// # Example
/// ```ignore
/// split!(net, reverb_id => output, analyzer_id);
/// split!(net, reverb_id => output, analyzer_id, meter_id);
/// ```
#[macro_export]
macro_rules! split {
    ($net:expr, $source:expr => output $(, $dest:expr)*) => {{
        $net.pipe_output($source);
        $(
            $net.pipe($source, $dest);
        )*
    }};

    ($net:expr, $source:expr => $first_dest:expr $(, $dest:expr)*) => {{
        $net.pipe($source, $first_dest);
        $(
            $net.pipe($source, $dest);
        )*
    }};
}
