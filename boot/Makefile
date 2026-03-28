.PHONY: clean, .force-rebuild
all: bootloader.bin

bootloader.bin: os.asm kernel.bin .force-rebuild
	nasm -fbin os.asm -o qi.img

kernel.bin: .force-rebuild
	cargo build 

clean:
	rm *.bin
