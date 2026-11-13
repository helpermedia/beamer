//! A struct with many parameters compiles, also in a release build.
//!
//! The parameters are listed in the order of the fields. A list built by
//! chaining one iterator per field nests a type per field, which the
//! compiler refuses past its recursion limit.

#![cfg(feature = "derive")]

use beamer::prelude::*;

#[beamer::group]
struct ManyGroup {
    #[parameter(id = "p0", name = "P0", default = 0.5, range = 0.0..=1.0)]
    p0: FloatParameter,

    #[parameter(id = "p1", name = "P1", default = 0.5, range = 0.0..=1.0)]
    p1: FloatParameter,

    #[parameter(id = "p2", name = "P2", default = 0.5, range = 0.0..=1.0)]
    p2: FloatParameter,

    #[parameter(id = "p3", name = "P3", default = 0.5, range = 0.0..=1.0)]
    p3: FloatParameter,

    #[parameter(id = "p4", name = "P4", default = 0.5, range = 0.0..=1.0)]
    p4: FloatParameter,

    #[parameter(id = "p5", name = "P5", default = 0.5, range = 0.0..=1.0)]
    p5: FloatParameter,

    #[parameter(id = "p6", name = "P6", default = 0.5, range = 0.0..=1.0)]
    p6: FloatParameter,

    #[parameter(id = "p7", name = "P7", default = 0.5, range = 0.0..=1.0)]
    p7: FloatParameter,

    #[parameter(id = "p8", name = "P8", default = 0.5, range = 0.0..=1.0)]
    p8: FloatParameter,

    #[parameter(id = "p9", name = "P9", default = 0.5, range = 0.0..=1.0)]
    p9: FloatParameter,

    #[parameter(id = "p10", name = "P10", default = 0.5, range = 0.0..=1.0)]
    p10: FloatParameter,

    #[parameter(id = "p11", name = "P11", default = 0.5, range = 0.0..=1.0)]
    p11: FloatParameter,

    #[parameter(id = "p12", name = "P12", default = 0.5, range = 0.0..=1.0)]
    p12: FloatParameter,

    #[parameter(id = "p13", name = "P13", default = 0.5, range = 0.0..=1.0)]
    p13: FloatParameter,

    #[parameter(id = "p14", name = "P14", default = 0.5, range = 0.0..=1.0)]
    p14: FloatParameter,

    #[parameter(id = "p15", name = "P15", default = 0.5, range = 0.0..=1.0)]
    p15: FloatParameter,

    #[parameter(id = "p16", name = "P16", default = 0.5, range = 0.0..=1.0)]
    p16: FloatParameter,

    #[parameter(id = "p17", name = "P17", default = 0.5, range = 0.0..=1.0)]
    p17: FloatParameter,

    #[parameter(id = "p18", name = "P18", default = 0.5, range = 0.0..=1.0)]
    p18: FloatParameter,

    #[parameter(id = "p19", name = "P19", default = 0.5, range = 0.0..=1.0)]
    p19: FloatParameter,

    #[parameter(id = "p20", name = "P20", default = 0.5, range = 0.0..=1.0)]
    p20: FloatParameter,

    #[parameter(id = "p21", name = "P21", default = 0.5, range = 0.0..=1.0)]
    p21: FloatParameter,

    #[parameter(id = "p22", name = "P22", default = 0.5, range = 0.0..=1.0)]
    p22: FloatParameter,

    #[parameter(id = "p23", name = "P23", default = 0.5, range = 0.0..=1.0)]
    p23: FloatParameter,

    #[parameter(id = "p24", name = "P24", default = 0.5, range = 0.0..=1.0)]
    p24: FloatParameter,

    #[parameter(id = "p25", name = "P25", default = 0.5, range = 0.0..=1.0)]
    p25: FloatParameter,

    #[parameter(id = "p26", name = "P26", default = 0.5, range = 0.0..=1.0)]
    p26: FloatParameter,

    #[parameter(id = "p27", name = "P27", default = 0.5, range = 0.0..=1.0)]
    p27: FloatParameter,

    #[parameter(id = "p28", name = "P28", default = 0.5, range = 0.0..=1.0)]
    p28: FloatParameter,

    #[parameter(id = "p29", name = "P29", default = 0.5, range = 0.0..=1.0)]
    p29: FloatParameter,

    #[parameter(id = "p30", name = "P30", default = 0.5, range = 0.0..=1.0)]
    p30: FloatParameter,

    #[parameter(id = "p31", name = "P31", default = 0.5, range = 0.0..=1.0)]
    p31: FloatParameter,

    #[parameter(id = "p32", name = "P32", default = 0.5, range = 0.0..=1.0)]
    p32: FloatParameter,

    #[parameter(id = "p33", name = "P33", default = 0.5, range = 0.0..=1.0)]
    p33: FloatParameter,

    #[parameter(id = "p34", name = "P34", default = 0.5, range = 0.0..=1.0)]
    p34: FloatParameter,

    #[parameter(id = "p35", name = "P35", default = 0.5, range = 0.0..=1.0)]
    p35: FloatParameter,

    #[parameter(id = "p36", name = "P36", default = 0.5, range = 0.0..=1.0)]
    p36: FloatParameter,

    #[parameter(id = "p37", name = "P37", default = 0.5, range = 0.0..=1.0)]
    p37: FloatParameter,

    #[parameter(id = "p38", name = "P38", default = 0.5, range = 0.0..=1.0)]
    p38: FloatParameter,

    #[parameter(id = "p39", name = "P39", default = 0.5, range = 0.0..=1.0)]
    p39: FloatParameter,

    #[parameter(id = "p40", name = "P40", default = 0.5, range = 0.0..=1.0)]
    p40: FloatParameter,

    #[parameter(id = "p41", name = "P41", default = 0.5, range = 0.0..=1.0)]
    p41: FloatParameter,

    #[parameter(id = "p42", name = "P42", default = 0.5, range = 0.0..=1.0)]
    p42: FloatParameter,

    #[parameter(id = "p43", name = "P43", default = 0.5, range = 0.0..=1.0)]
    p43: FloatParameter,

    #[parameter(id = "p44", name = "P44", default = 0.5, range = 0.0..=1.0)]
    p44: FloatParameter,

    #[parameter(id = "p45", name = "P45", default = 0.5, range = 0.0..=1.0)]
    p45: FloatParameter,

    #[parameter(id = "p46", name = "P46", default = 0.5, range = 0.0..=1.0)]
    p46: FloatParameter,

    #[parameter(id = "p47", name = "P47", default = 0.5, range = 0.0..=1.0)]
    p47: FloatParameter,

    #[parameter(id = "p48", name = "P48", default = 0.5, range = 0.0..=1.0)]
    p48: FloatParameter,

    #[parameter(id = "p49", name = "P49", default = 0.5, range = 0.0..=1.0)]
    p49: FloatParameter,

    #[parameter(id = "p50", name = "P50", default = 0.5, range = 0.0..=1.0)]
    p50: FloatParameter,

    #[parameter(id = "p51", name = "P51", default = 0.5, range = 0.0..=1.0)]
    p51: FloatParameter,

    #[parameter(id = "p52", name = "P52", default = 0.5, range = 0.0..=1.0)]
    p52: FloatParameter,

    #[parameter(id = "p53", name = "P53", default = 0.5, range = 0.0..=1.0)]
    p53: FloatParameter,

    #[parameter(id = "p54", name = "P54", default = 0.5, range = 0.0..=1.0)]
    p54: FloatParameter,

    #[parameter(id = "p55", name = "P55", default = 0.5, range = 0.0..=1.0)]
    p55: FloatParameter,

    #[parameter(id = "p56", name = "P56", default = 0.5, range = 0.0..=1.0)]
    p56: FloatParameter,

    #[parameter(id = "p57", name = "P57", default = 0.5, range = 0.0..=1.0)]
    p57: FloatParameter,

    #[parameter(id = "p58", name = "P58", default = 0.5, range = 0.0..=1.0)]
    p58: FloatParameter,

    #[parameter(id = "p59", name = "P59", default = 0.5, range = 0.0..=1.0)]
    p59: FloatParameter,

    #[parameter(id = "p60", name = "P60", default = 0.5, range = 0.0..=1.0)]
    p60: FloatParameter,

    #[parameter(id = "p61", name = "P61", default = 0.5, range = 0.0..=1.0)]
    p61: FloatParameter,

    #[parameter(id = "p62", name = "P62", default = 0.5, range = 0.0..=1.0)]
    p62: FloatParameter,

    #[parameter(id = "p63", name = "P63", default = 0.5, range = 0.0..=1.0)]
    p63: FloatParameter,

    #[parameter(id = "p64", name = "P64", default = 0.5, range = 0.0..=1.0)]
    p64: FloatParameter,

    #[parameter(id = "p65", name = "P65", default = 0.5, range = 0.0..=1.0)]
    p65: FloatParameter,

    #[parameter(id = "p66", name = "P66", default = 0.5, range = 0.0..=1.0)]
    p66: FloatParameter,

    #[parameter(id = "p67", name = "P67", default = 0.5, range = 0.0..=1.0)]
    p67: FloatParameter,

    #[parameter(id = "p68", name = "P68", default = 0.5, range = 0.0..=1.0)]
    p68: FloatParameter,

    #[parameter(id = "p69", name = "P69", default = 0.5, range = 0.0..=1.0)]
    p69: FloatParameter,

    #[parameter(id = "p70", name = "P70", default = 0.5, range = 0.0..=1.0)]
    p70: FloatParameter,

    #[parameter(id = "p71", name = "P71", default = 0.5, range = 0.0..=1.0)]
    p71: FloatParameter,

    #[parameter(id = "p72", name = "P72", default = 0.5, range = 0.0..=1.0)]
    p72: FloatParameter,

    #[parameter(id = "p73", name = "P73", default = 0.5, range = 0.0..=1.0)]
    p73: FloatParameter,

    #[parameter(id = "p74", name = "P74", default = 0.5, range = 0.0..=1.0)]
    p74: FloatParameter,

    #[parameter(id = "p75", name = "P75", default = 0.5, range = 0.0..=1.0)]
    p75: FloatParameter,

    #[parameter(id = "p76", name = "P76", default = 0.5, range = 0.0..=1.0)]
    p76: FloatParameter,

    #[parameter(id = "p77", name = "P77", default = 0.5, range = 0.0..=1.0)]
    p77: FloatParameter,

    #[parameter(id = "p78", name = "P78", default = 0.5, range = 0.0..=1.0)]
    p78: FloatParameter,

    #[parameter(id = "p79", name = "P79", default = 0.5, range = 0.0..=1.0)]
    p79: FloatParameter,

    #[parameter(id = "p80", name = "P80", default = 0.5, range = 0.0..=1.0)]
    p80: FloatParameter,

    #[parameter(id = "p81", name = "P81", default = 0.5, range = 0.0..=1.0)]
    p81: FloatParameter,

    #[parameter(id = "p82", name = "P82", default = 0.5, range = 0.0..=1.0)]
    p82: FloatParameter,

    #[parameter(id = "p83", name = "P83", default = 0.5, range = 0.0..=1.0)]
    p83: FloatParameter,

    #[parameter(id = "p84", name = "P84", default = 0.5, range = 0.0..=1.0)]
    p84: FloatParameter,

    #[parameter(id = "p85", name = "P85", default = 0.5, range = 0.0..=1.0)]
    p85: FloatParameter,

    #[parameter(id = "p86", name = "P86", default = 0.5, range = 0.0..=1.0)]
    p86: FloatParameter,

    #[parameter(id = "p87", name = "P87", default = 0.5, range = 0.0..=1.0)]
    p87: FloatParameter,

    #[parameter(id = "p88", name = "P88", default = 0.5, range = 0.0..=1.0)]
    p88: FloatParameter,

    #[parameter(id = "p89", name = "P89", default = 0.5, range = 0.0..=1.0)]
    p89: FloatParameter,

    #[parameter(id = "p90", name = "P90", default = 0.5, range = 0.0..=1.0)]
    p90: FloatParameter,

    #[parameter(id = "p91", name = "P91", default = 0.5, range = 0.0..=1.0)]
    p91: FloatParameter,

    #[parameter(id = "p92", name = "P92", default = 0.5, range = 0.0..=1.0)]
    p92: FloatParameter,

    #[parameter(id = "p93", name = "P93", default = 0.5, range = 0.0..=1.0)]
    p93: FloatParameter,

    #[parameter(id = "p94", name = "P94", default = 0.5, range = 0.0..=1.0)]
    p94: FloatParameter,

    #[parameter(id = "p95", name = "P95", default = 0.5, range = 0.0..=1.0)]
    p95: FloatParameter,

    #[parameter(id = "p96", name = "P96", default = 0.5, range = 0.0..=1.0)]
    p96: FloatParameter,

    #[parameter(id = "p97", name = "P97", default = 0.5, range = 0.0..=1.0)]
    p97: FloatParameter,

    #[parameter(id = "p98", name = "P98", default = 0.5, range = 0.0..=1.0)]
    p98: FloatParameter,

    #[parameter(id = "p99", name = "P99", default = 0.5, range = 0.0..=1.0)]
    p99: FloatParameter,

    #[parameter(id = "p100", name = "P100", default = 0.5, range = 0.0..=1.0)]
    p100: FloatParameter,

    #[parameter(id = "p101", name = "P101", default = 0.5, range = 0.0..=1.0)]
    p101: FloatParameter,

    #[parameter(id = "p102", name = "P102", default = 0.5, range = 0.0..=1.0)]
    p102: FloatParameter,

    #[parameter(id = "p103", name = "P103", default = 0.5, range = 0.0..=1.0)]
    p103: FloatParameter,

    #[parameter(id = "p104", name = "P104", default = 0.5, range = 0.0..=1.0)]
    p104: FloatParameter,

    #[parameter(id = "p105", name = "P105", default = 0.5, range = 0.0..=1.0)]
    p105: FloatParameter,

    #[parameter(id = "p106", name = "P106", default = 0.5, range = 0.0..=1.0)]
    p106: FloatParameter,

    #[parameter(id = "p107", name = "P107", default = 0.5, range = 0.0..=1.0)]
    p107: FloatParameter,

    #[parameter(id = "p108", name = "P108", default = 0.5, range = 0.0..=1.0)]
    p108: FloatParameter,

    #[parameter(id = "p109", name = "P109", default = 0.5, range = 0.0..=1.0)]
    p109: FloatParameter,

    #[parameter(id = "p110", name = "P110", default = 0.5, range = 0.0..=1.0)]
    p110: FloatParameter,

    #[parameter(id = "p111", name = "P111", default = 0.5, range = 0.0..=1.0)]
    p111: FloatParameter,

    #[parameter(id = "p112", name = "P112", default = 0.5, range = 0.0..=1.0)]
    p112: FloatParameter,

    #[parameter(id = "p113", name = "P113", default = 0.5, range = 0.0..=1.0)]
    p113: FloatParameter,

    #[parameter(id = "p114", name = "P114", default = 0.5, range = 0.0..=1.0)]
    p114: FloatParameter,

    #[parameter(id = "p115", name = "P115", default = 0.5, range = 0.0..=1.0)]
    p115: FloatParameter,

    #[parameter(id = "p116", name = "P116", default = 0.5, range = 0.0..=1.0)]
    p116: FloatParameter,

    #[parameter(id = "p117", name = "P117", default = 0.5, range = 0.0..=1.0)]
    p117: FloatParameter,

    #[parameter(id = "p118", name = "P118", default = 0.5, range = 0.0..=1.0)]
    p118: FloatParameter,

    #[parameter(id = "p119", name = "P119", default = 0.5, range = 0.0..=1.0)]
    p119: FloatParameter,

    #[parameter(id = "p120", name = "P120", default = 0.5, range = 0.0..=1.0)]
    p120: FloatParameter,

    #[parameter(id = "p121", name = "P121", default = 0.5, range = 0.0..=1.0)]
    p121: FloatParameter,

    #[parameter(id = "p122", name = "P122", default = 0.5, range = 0.0..=1.0)]
    p122: FloatParameter,

    #[parameter(id = "p123", name = "P123", default = 0.5, range = 0.0..=1.0)]
    p123: FloatParameter,

    #[parameter(id = "p124", name = "P124", default = 0.5, range = 0.0..=1.0)]
    p124: FloatParameter,

    #[parameter(id = "p125", name = "P125", default = 0.5, range = 0.0..=1.0)]
    p125: FloatParameter,

    #[parameter(id = "p126", name = "P126", default = 0.5, range = 0.0..=1.0)]
    p126: FloatParameter,

    #[parameter(id = "p127", name = "P127", default = 0.5, range = 0.0..=1.0)]
    p127: FloatParameter,

    #[parameter(id = "p128", name = "P128", default = 0.5, range = 0.0..=1.0)]
    p128: FloatParameter,

    #[parameter(id = "p129", name = "P129", default = 0.5, range = 0.0..=1.0)]
    p129: FloatParameter,

    #[parameter(id = "p130", name = "P130", default = 0.5, range = 0.0..=1.0)]
    p130: FloatParameter,

    #[parameter(id = "p131", name = "P131", default = 0.5, range = 0.0..=1.0)]
    p131: FloatParameter,

    #[parameter(id = "p132", name = "P132", default = 0.5, range = 0.0..=1.0)]
    p132: FloatParameter,

    #[parameter(id = "p133", name = "P133", default = 0.5, range = 0.0..=1.0)]
    p133: FloatParameter,

    #[parameter(id = "p134", name = "P134", default = 0.5, range = 0.0..=1.0)]
    p134: FloatParameter,

    #[parameter(id = "p135", name = "P135", default = 0.5, range = 0.0..=1.0)]
    p135: FloatParameter,

    #[parameter(id = "p136", name = "P136", default = 0.5, range = 0.0..=1.0)]
    p136: FloatParameter,

    #[parameter(id = "p137", name = "P137", default = 0.5, range = 0.0..=1.0)]
    p137: FloatParameter,

    #[parameter(id = "p138", name = "P138", default = 0.5, range = 0.0..=1.0)]
    p138: FloatParameter,

    #[parameter(id = "p139", name = "P139", default = 0.5, range = 0.0..=1.0)]
    p139: FloatParameter,

    #[parameter(id = "p140", name = "P140", default = 0.5, range = 0.0..=1.0)]
    p140: FloatParameter,

    #[parameter(id = "p141", name = "P141", default = 0.5, range = 0.0..=1.0)]
    p141: FloatParameter,

    #[parameter(id = "p142", name = "P142", default = 0.5, range = 0.0..=1.0)]
    p142: FloatParameter,

    #[parameter(id = "p143", name = "P143", default = 0.5, range = 0.0..=1.0)]
    p143: FloatParameter,

    #[parameter(id = "p144", name = "P144", default = 0.5, range = 0.0..=1.0)]
    p144: FloatParameter,

    #[parameter(id = "p145", name = "P145", default = 0.5, range = 0.0..=1.0)]
    p145: FloatParameter,

    #[parameter(id = "p146", name = "P146", default = 0.5, range = 0.0..=1.0)]
    p146: FloatParameter,

    #[parameter(id = "p147", name = "P147", default = 0.5, range = 0.0..=1.0)]
    p147: FloatParameter,

    #[parameter(id = "p148", name = "P148", default = 0.5, range = 0.0..=1.0)]
    p148: FloatParameter,

    #[parameter(id = "p149", name = "P149", default = 0.5, range = 0.0..=1.0)]
    p149: FloatParameter,
}

#[test]
fn test_many_parameters_in_one_struct() {
    let group = ManyGroup::default();

    let paths: Vec<String> = group.iter().map(|parameter| parameter.info().string_id.to_string()).collect();
    let expected: Vec<String> = (0..150).map(|index| format!("p{index}")).collect();
    assert_eq!(paths, expected);
}
