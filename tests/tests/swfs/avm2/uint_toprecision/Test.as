﻿package {
	public class Test {
	}
}

function assert_exp(val: uint) {
	trace("///(digits = 1)");
	trace(val.toPrecision(1));
	trace("///(digits = 2)");
	trace(val.toPrecision(2));
	trace("///(digits = 3)");
	trace(val.toPrecision(3));
	trace("///(digits = 4)");
	trace(val.toPrecision(4));
	trace("///(digits = 5)");
	trace(val.toPrecision(5));
	trace("///(digits = 6)");
	trace(val.toPrecision(6));
	trace("///(digits = 7)");
	trace(val.toPrecision(7));
	trace("///(digits = 8)");
	trace(val.toPrecision(8));
	trace("///(digits = 9)");
	trace(val.toPrecision(9));
	trace("///(digits = 10)");
	trace(val.toPrecision(10));
	trace("///(digits = 20)");
	trace(val.toPrecision(20));
	trace("///(digits = 21)");
	trace(val.toPrecision(21));
}

trace("//true");
assert_exp(true);

trace("//false");
assert_exp(false);

trace("//null");
assert_exp(null);

trace("//undefined");
assert_exp(undefined);

trace("//\"\"");
assert_exp("");

trace("//\"str\"");
assert_exp("str");

trace("//\"true\"");
assert_exp("true");

trace("//\"false\"");
assert_exp("false");

trace("//0.0");
assert_exp(0.0);

trace("//NaN");
assert_exp(NaN);

trace("//-0.0");
assert_exp(-0.0);

trace("//Infinity");
assert_exp(Infinity);

trace("//1.0");
assert_exp(1.0);

trace("//-1.0");
assert_exp(-1.0);

trace("//0xFF1306");
assert_exp(0xFF1306);

trace("//1.2315e2");
assert_exp(1.2315e2);

trace("//0x7FFFFFFF");
assert_exp(0x7FFFFFFF);

trace("//0x80000000");
assert_exp(0x80000000);

trace("//0x80000001");
assert_exp(0x80000001);

trace("//0x180000001");
assert_exp(0x180000001);

trace("//0x100000001");
assert_exp(0x100000001);

trace("//-0x7FFFFFFF");
assert_exp(-0x7FFFFFFF);

trace("//-0x80000000");
assert_exp(-0x80000000);

trace("//-0x80000001");
assert_exp(-0x80000001);

trace("//-0x180000001");
assert_exp(-0x180000001);

trace("//-0x100000001");
assert_exp(-0x100000001);

trace("//new Object()");
assert_exp({});

trace("//\"0.0\"");
assert_exp("0.0");

trace("//\"NaN\"");
assert_exp("NaN");

trace("//\"-0.0\"");
assert_exp("-0.0");

trace("//\"Infinity\"");
assert_exp("Infinity");

trace("//\"1.0\"");
assert_exp("1.0");

trace("//\"-1.0\"");
assert_exp("-1.0");

trace("//\"0xFF1306\"");
assert_exp("0xFF1306");

trace("//\"1.2315e2\"");
assert_exp("1.2315e2");

trace("//\"0x7FFFFFFF\"");
assert_exp(0x7FFFFFFF);

trace("//\"0x80000000\"");
assert_exp(0x80000000);

trace("//\"0x80000001\"");
assert_exp(0x80000001);

trace("//\"0x180000001\"");
assert_exp(0x180000001);

trace("//\"0x100000001\"");
assert_exp(0x100000001);

trace("//\"-0x7FFFFFFF\"");
assert_exp(-0x7FFFFFFF);

trace("//\"-0x80000000\"");
assert_exp(-0x80000000);

trace("//\"-0x80000001\"");
assert_exp(-0x80000001);

trace("//\"-0x180000001\"");
assert_exp(-0x180000001);

trace("//\"-0x100000001\"");
assert_exp(-0x100000001);