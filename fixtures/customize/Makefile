.PHONY: all
all: $(patsubst %.bu,%.ign,$(wildcard *.bu))

%.ign: %.bu
	butane -p $< -o $@
