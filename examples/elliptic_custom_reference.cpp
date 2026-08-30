#include "elliptic-blep.h"

#include <cmath>
#include <cstdint>
#include <fstream>
#include <iostream>

static double curve(double phase) {
	if (phase < 0.25) return phase*4;
	if (phase < 0.75) return 2 - phase*4;
	return phase*4 - 4;
}

int main(int argc, char **argv) {
	if (argc != 4) return 2;
	const uint64_t bin = std::stoull(argv[1]);
	const uint64_t samples = std::stoull(argv[2]);
	const double step = double(bin)/samples;
	signalsmith::blep::EllipticBlep<double> blep;
	signalsmith::blep::EllipticBlepAllpass<double> allpass;
	double phase = 0;
	std::ofstream output(argv[3], std::ios::binary);
	for (uint64_t frame = 0; frame < samples*5; ++frame) {
		double next = phase + step;
		blep.step();
		for (auto [event, jump] : {std::pair{0.25, -8.0}, std::pair{0.75, 8.0}}) {
			double distance = event - phase;
			if (distance <= 0) distance += 1;
			if (distance <= step) blep.add(jump*step, 2, (step - distance)/step);
		}
		phase = next - std::floor(next);
		double sample = allpass(curve(phase) + blep.get());
		if (frame >= samples*4) {
			float value = sample;
			output.write(reinterpret_cast<const char *>(&value), sizeof(value));
		}
	}
	std::cout << "linear_delay=" << signalsmith::blep::EllipticBlepAllpass<double>::linearDelay << "\n";
}
