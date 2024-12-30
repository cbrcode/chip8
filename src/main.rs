// CHIP8 Emulator by Christian Barton Randall
// Reference: https://www.cs.columbia.edu/~sedwards/classes/2016/4840-spring/designs/Chip8.pdf

#![deny(clippy::all)]
#![forbid(unsafe_code)]
#![allow(dead_code)]

use error_iter::ErrorIter as _;
use log::error;
use pixels::{Error, Pixels, SurfaceTexture};
use winit::dpi::LogicalSize;
use winit::event::{Event, WindowEvent};
use winit::event_loop::EventLoop;
use winit::keyboard::KeyCode;
use winit::window::WindowBuilder;
use winit_input_helper::WinitInputHelper;
use std::{fs, time::Instant};
use rand::Rng;

// Chip 8 resolution is 64x32 so we upscale this by a factor of k
const K: u32 = 4; // upscaling factor
const WIDTH: u32 = 64;
const HEIGHT: u32 = 32;
const INSTRUCTIONS_PER_SECOND: usize = 700; // the amount of instructions to execute per second

const SUPER_CHIP: bool = true; // if ROM doesn't work, try messing around with this 

/*
All setting of pixels of this display are done through the use of sprites that are always 8 × N where N is the pixel height
of the sprite. Chip8 comes with a font set (sprites) that allows character 0-9 and A-F to be printed directly to the
screen. Each one of these characters fit within a 8x5 grid.
*/

fn get_bit(value: &u8, position: &u8) -> bool { // from most to least significant
    value & (1 << (7-position)) != 0
}

/*
The framebuffer is a 64x32 bit memory array that is written two in 8-bit chunks by reading memory locations. The framebuffer
will feature a wraparound that causes the pixels to be written from the position Y + 0 in the y axis, all the way until (Y +N)%32.
This will allow for proper wraparound of the sprites that need to be drawn.
*/
struct FrameBuffer {
    pixels: [[bool; 64]; 32] // 32 rows of 64
}

impl FrameBuffer {
    fn new() -> Self {
        Self {
            pixels: [[false; 64]; 32],
        }
    }

    fn clear(&mut self) {
        self.pixels = [[false; 64]; 32];
    }

    fn set(&mut self, x: u8, y: u8, value: u8) -> bool {
        let mut vf_flip = false;

        for i in 0..8 {
            if x + i > 63 {
                break;
            }
            let bit_on = get_bit(&value, &i);
            if bit_on { // if the bit was on before and it's getting turned off, flip VF
                if self.pixels[y as usize][(x + i) as usize] {
                    vf_flip = true; 
                }
                self.pixels[y as usize][(x + i) as usize] ^= true;
            }
        }
        
        vf_flip
    }

    fn export(&self) -> [bool; 2048] {
        let mut final_array = [false; 2048];
        for j in 0..HEIGHT {
            for i in 0..WIDTH {
                final_array[(j*WIDTH+i) as usize] = self.pixels[j as usize][i as usize];
            }
        }

        final_array
    }
}

// Representation of the application state. In this example, a box will bounce around the screen.
struct CHIP8 {
    registers: [u8; 16],
    memory: [u8; 4096],  // index 512 (0x200) to 4095 are the program memory, 0x00 to 0x80 is
                         // supposed to be the default font storage
    stack: Vec<u16>,     // 64 byte stack

    pc: u16,             // Program Counter
    sp: u16,             // Stack Pointer
    index_reg: u16,
    current_op: String,     // Current OP Code
    
    sound_timer: u8,
    delay_timer: u8,

    frame_buffer: FrameBuffer,

    paused: bool,

    key_pressed: bool,
    last_key: Option<u8>,

    last_instant: Instant,
}

/* Font
0xF0, 0x90, 0x90, 0x90, 0xF0, // 0
0x20, 0x60, 0x20, 0x20, 0x70, // 1
0xF0, 0x10, 0xF0, 0x80, 0xF0, // 2
0xF0, 0x10, 0xF0, 0x10, 0xF0, // 3
0x90, 0x90, 0xF0, 0x10, 0x10, // 4
0xF0, 0x80, 0xF0, 0x10, 0xF0, // 5
0xF0, 0x80, 0xF0, 0x90, 0xF0, // 6
0xF0, 0x10, 0x20, 0x40, 0x40, // 7
0xF0, 0x90, 0xF0, 0x90, 0xF0, // 8
0xF0, 0x90, 0xF0, 0x10, 0xF0, // 9
0xF0, 0x90, 0xF0, 0x90, 0x90, // A
0xE0, 0x90, 0xE0, 0x90, 0xE0, // B
0xF0, 0x80, 0x80, 0x80, 0xF0, // C
0xE0, 0x90, 0x90, 0x90, 0xE0, // D
0xF0, 0x80, 0xF0, 0x80, 0xF0, // E
0xF0, 0x80, 0xF0, 0x80, 0x80  // F
*/

// borrows memory and inserts fontset into the first 512 indices

fn get_character_sprite(c: char) -> [u8; 5] {
    match c {
        '0' => [0xF0, 0x90, 0x90, 0x90, 0xF0],
        '1' => [0x20, 0x60, 0x20, 0x20, 0x70],
        '2' => [0xF0, 0x10, 0xF0, 0x80, 0xF0],
        '3' => [0xF0, 0x10, 0xF0, 0x10, 0xF0],
        '4' => [0x90, 0x90, 0xF0, 0x10, 0x10],
        '5' => [0xF0, 0x80, 0xF0, 0x10, 0xF0],
        '6' => [0xF0, 0x80, 0xF0, 0x90, 0xF0],
        '7' => [0xF0, 0x10, 0x20, 0x40, 0x40],
        '8' => [0xF0, 0x90, 0xF0, 0x90, 0xF0],
        '9' => [0xF0, 0x90, 0xF0, 0x10, 0xF0],
        'A' => [0xF0, 0x90, 0xF0, 0x90, 0x90],
        'B' => [0xE0, 0x90, 0xE0, 0x90, 0xE0],
        'C' => [0xF0, 0x80, 0x80, 0x80, 0xF0],
        'D' => [0xE0, 0x90, 0x90, 0x90, 0xE0],
        'E' => [0xF0, 0x80, 0xF0, 0x80, 0xF0],
        'F' => [0xF0, 0x80, 0xF0, 0x80, 0x80],
        _ => [0, 0, 0, 0, 0],
    }
}

fn load_font_into_memory(memory: &mut [u8; 4096]) {
    let mut i = 0;
    for c in '0'..='9' {
        for j in get_character_sprite(c) {
            memory[i] = j;
            i += 1;
        }
    }

    for c in 'A'..='F' {
        for j in get_character_sprite(c) {
            memory[i] = j;
            i += 1;
        }
    }
}

fn load_program_into_memory(memory: &mut [u8; 4096], program: Vec<u8>) {
    let mut index: usize = 512;
    for i in program {
        memory[index] = i;
        index += 1;
    }
}

impl CHIP8 {
    /// Create a new emulator
    fn new(program: Vec<u8>) -> Self {
        let mut memory: [u8; 4096] = [0; 4096];

        load_font_into_memory(&mut memory);
        load_program_into_memory(&mut memory, program);

        Self {
            registers: [0; 16],
            memory,

            pc: 512,
            sp: 0,
            index_reg: 0,
            stack: vec![0; 32],
            current_op: String::from(""),

            sound_timer: 0,
            delay_timer: 0,

            frame_buffer: FrameBuffer::new(),

            paused: false,

            key_pressed: false,
            last_key: None,

            last_instant: Instant::now(),
        }
    }

    fn pause(&mut self) {
        self.paused ^= true;
    }

    /// Update the `World` internal state; bounce the box around the screen.
    fn update(&mut self) {
        // fetch decode execute
        
        // fetch
        
        if self.paused {
            return;
        }

        self.current_op = format!("{:X}", self.memory[self.pc as usize]);
        // println!("BEFORE: {}", self.current_op);
        if self.current_op.len() == 1 {
            self.current_op = format!("0{}", self.current_op);
        }
        self.current_op = format!("{}{:X}", self.current_op, self.memory[(self.pc + 1) as usize]);
        if self.current_op.len() == 3 {
            self.current_op.insert(2, '0');
        }
        // println!("AFTER: {}", self.current_op);
        self.process_op();
    }

    fn process_op(&mut self) {
        let op: String = self.current_op.clone(); // convert op to hexadecimal string slice
        println!("INSTRUCTION: {}", op);
        let chars: Vec<char> = op.chars().collect();              // collect the slice into a vec of chars
        
        let mut inc = true; // determine if you increment the program counter

        match chars[0] {
            '0' => if chars[2] == 'E' { match chars[3] {
                '0' => self.frame_buffer.clear(),
                'E' => self.return_from_subroutine(),
                _ => {},
            }}, // only needs to handle 00E0 & 00EE
            '1' => { // JMP
                self.pc = self.hex_chars_to_u16(chars[1..].to_vec());
                println!("JMP to {}", self.pc);
                inc = false;
            },
            '2' => {
                self.stack.push(self.pc);
                self.pc = self.hex_chars_to_u16(chars[1..].to_vec());
            },
            '3' => {
                if self.registers[self.hex_char_to_u16(chars[1]) as usize] as u16 == self.hex_chars_to_u16(chars[2..].to_vec()) {
                    self.pc += 2; // skip next instruction
                }
            },
            '4' => {
                if self.registers[self.hex_char_to_u16(chars[1]) as usize] as u16 != self.hex_chars_to_u16(chars[2..].to_vec()) {
                    self.pc += 2; // skip next instruction
                }
            },
            '5' => if chars[3] == '0' && self.registers[self.hex_char_to_u16(chars[1]) as usize] == self.registers[self.hex_char_to_u16(chars[1]) as usize] {
                self.pc += 2; // skip next instruction
            },
            '6' => {
                let index = self.hex_char_to_u16(chars[1]);
                println!("SETTING REGISTER V{:X} to {}", index, self.hex_chars_to_u16(chars[2..].to_vec()));
                self.registers[index as usize] = self.hex_chars_to_u16(chars[2..].to_vec()) as u8;
            }, // LDR
            '7' => {
                let index = self.hex_char_to_u16(chars[1]);
                println!("ADDING TO REGISTER V{:X}", index);
                if (255 - self.registers[index as usize] as u16) >= self.hex_chars_to_u16(chars[2..].to_vec()) {
                    self.registers[index as usize] += self.hex_chars_to_u16(chars[2..].to_vec()) as u8;
                } else {
                    self.registers[index as usize] = 255;
                }
            },
            '8' => { println!("!!!!!!!!!!! CHARS 3: {}", chars[3]); match chars[3] {
                '0' => { // assignment
                    self.registers[self.hex_char_to_u16(chars[1]) as usize] = self.registers[self.hex_char_to_u16(chars[2]) as usize];
                },
                '1' => { // bitwise or
                    self.registers[self.hex_char_to_u16(chars[1]) as usize] |= self.registers[self.hex_char_to_u16(chars[2]) as usize];
                },
                '2' => { // bitwise and
                    self.registers[self.hex_char_to_u16(chars[1]) as usize] &= self.registers[self.hex_char_to_u16(chars[2]) as usize];
                },
                '3' => { // bitwise xor
                    self.registers[self.hex_char_to_u16(chars[1]) as usize] ^= self.registers[self.hex_char_to_u16(chars[2]) as usize];
                },
                '4' => {
                    let index = self.hex_char_to_u16(chars[1]);
                    let second_index = self.hex_char_to_u16(chars[2]);
                    if 255 - self.registers[index as usize] >= self.registers[second_index as usize] {
                        self.registers[index as usize] += self.registers[second_index as usize];
                        self.registers[15] = 0;
                    } else {
                        self.registers[index as usize] = 255;
                        self.registers[15] = 1;
                    }
                }, // add (with carry flag)
                '5' => { // subtract VX - VY into VX
                    let index = self.hex_char_to_u16(chars[1]);
                    let second_index = self.hex_char_to_u16(chars[2]);
                    if self.registers[index as usize] > self.registers[second_index as usize] {
                        println!("SUBTRACTING V{} - V{}", index, second_index);
                        println!("{} - {} =", self.registers[index as usize], self.registers[second_index as usize]);
                        self.registers[index as usize] -= self.registers[second_index as usize];
                        println!("{}", self.registers[index as usize]);
                        self.registers[15] = 1;
                    } else {
                        self.registers[index as usize] = 0;
                        self.registers[15] = 0;
                    }
                },
                '6' => { // bitwise right
                    if SUPER_CHIP {
                        self.registers[self.hex_char_to_u16(chars[1]) as usize] = self.registers[self.hex_char_to_u16(chars[2]) as usize];
                    }
                    if get_bit(&self.registers[self.hex_char_to_u16(chars[1]) as usize], &7) {
                        self.registers[15] = 1;
                    } else {
                        self.registers[15] = 0;
                    }
                    self.registers[self.hex_char_to_u16(chars[1]) as usize] = self.registers[self.hex_char_to_u16(chars[1]) as usize] >> 1;
                },
                '7' => { // subtract VY - VX into VX
                    let index = self.hex_char_to_u16(chars[1]);
                    let second_index = self.hex_char_to_u16(chars[2]);

                    println!("!!!!!!!!!!! EIGHT SEVEN");

                    if self.registers[second_index as usize] > self.registers[index as usize] {
                        println!("SUBTRACTING V{} - V{}", second_index, index);
                        println!("{} - {} =", self.registers[second_index as usize], self.registers[index as usize]);
                        self.registers[index as usize] = self.registers[second_index as usize] - self.registers[index as usize];
                        println!("{}", self.registers[index as usize]);
                        self.registers[15] = 1;
                    } else {
                        println!("SUBTRACTING V{} - V{}", second_index, index);
                        self.registers[index as usize] = 0;
                        self.registers[15] = 0;
                    }
                },
                'E' => { // bitwise left
                    if SUPER_CHIP {
                        self.registers[self.hex_char_to_u16(chars[1]) as usize] = self.registers[self.hex_char_to_u16(chars[2]) as usize];
                    }
                    if get_bit(&self.registers[self.hex_char_to_u16(chars[1]) as usize], &0) {
                        self.registers[15] = 1;
                    } else {
                        self.registers[15] = 0;
                    }
                    self.registers[self.hex_char_to_u16(chars[1]) as usize] = self.registers[self.hex_char_to_u16(chars[1]) as usize] << 1;
                },
                _ => {},
            }},
            '9' => if chars[3] == '0' && self.registers[self.hex_char_to_u16(chars[1]) as usize] != self.registers[self.hex_char_to_u16(chars[1]) as usize] {
                self.pc += 2; // skip next instruction
            },
            'A' => {
                self.index_reg = self.hex_chars_to_u16(chars[1..].to_vec());
                println!("SETTINGS INDEX REGISTER TO {}", self.index_reg);
            }, // SET INDEX REG
            'B' => {
                self.pc = self.hex_chars_to_u16(chars[1..].to_vec());
                let mut reg = 0;
                if SUPER_CHIP {
                    reg = self.hex_char_to_u16(chars[1]);
                }
                self.pc += self.registers[reg as usize] as u16;
                inc = false;
            },
            'C' => {
                let last_two = self.hex_chars_to_u16(chars[2..].to_vec());
                self.registers[self.hex_char_to_u16(chars[1]) as usize] = (rand::thread_rng().gen_range(0..last_two) & last_two) as u8;
            },
            'D' => {
                let x = self.registers[self.hex_char_to_u16(chars[1]) as usize] % 64;
                let mut y = self.registers[self.hex_char_to_u16(chars[2]) as usize] % 32;
                let n = self.hex_char_to_u16(chars[3]);

                let mut vf_flip = false;
                
                for i in 0..n {
                    if y > 31 {
                        break;
                    }
                    let location = self.index_reg + i; // 8 bits
                    vf_flip = self.frame_buffer.set(x,y, self.memory[location as usize]);
                    y += 1;
                }

                if vf_flip {
                    self.registers[15] = 1;
                } else {
                    self.registers[15] = 0;
                }
                
            }, // Fun stuff (drawing)
            'E' => if self.key_pressed { match chars[2..3] {
                ['9', 'E'] => {
                    if self.registers[self.hex_char_to_u16(chars[1]) as usize] == self.last_key.unwrap() {
                        self.pc += 2;
                    }
                },
                ['A', '1'] => {
                    if self.registers[self.hex_char_to_u16(chars[1]) as usize] != self.last_key.unwrap() {
                        self.pc += 2;
                    }
                },
                _ => {},
            }},
            'F' => match chars[2..3] {
                ['0', '7'] => {
                    self.registers[self.hex_char_to_u16(chars[1]) as usize] = self.delay_timer;
                },

                ['1', '5'] => {
                    self.delay_timer = self.registers[self.hex_char_to_u16(chars[1]) as usize];
                },

                ['1', '8'] => {
                    self.sound_timer = self.registers[self.hex_char_to_u16(chars[1]) as usize];
                },

                ['1', 'E'] => { // Add to index register (Spacefight 2091! ROM relies on carry flag behaviour that's commented out here)
                    self.index_reg += self.registers[self.hex_char_to_u16(chars[1]) as usize] as u16; // shouldn't need to handle index register overflow
                    // if self.index_reg > 0x0FF { // over 12-bit
                    //     self.registers[15] = 1;
                    // }
                },

                ['0', 'A'] => {
                    if self.key_pressed {
                        self.registers[self.hex_char_to_u16(chars[1]) as usize] = self.last_key.unwrap();
                    } else {
                        self.pc -= 2; // decrement program counter to come back here until key is pressed
                    }
                },

                ['2', '9'] => { // Font character
                    let character = (self.registers[self.hex_char_to_u16(chars[1]) as usize] % 16) as u16;
                    self.index_reg = character * 5; // 5 rows or bytes in each letter sprite
                },

                ['3', '3'] => { // Splice register value by the units, tens, hundreds into memory starting at the index register
                    let number = self.registers[self.hex_char_to_u16(chars[1]) as usize];
                    let digit_three = number % 10;
                    let digit_two = (number % 100 - digit_three) / 10;
                    let digit_one = (number - digit_two*10 - digit_three) / 100; 

                    self.memory[self.index_reg as usize] = digit_one;
                    self.memory[(self.index_reg + 1) as usize] = digit_two;
                    self.memory[(self.index_reg + 2) as usize] = digit_three;
                },

                ['5', '5'] => { // V0 -> VX gets loaded with memory starting at index register
                    let max = self.hex_char_to_u16(chars[1]);
                    for i in 0..=max {
                        self.memory[(self.index_reg + i) as usize] = self.registers[i as usize];
                    }

                    if !SUPER_CHIP { // older interpreters incremented index registers as they worked
                        self.index_reg += self.hex_char_to_u16(chars[1]) + 1;
                    }
                },

                ['6', '5'] => { // memory starting at index register gets loaded with V0 -> VX
                    let max = self.hex_char_to_u16(chars[1]);
                    for i in 0..=max {
                        self.registers[i as usize] = self.memory[(self.index_reg + i) as usize];
                    }

                    if !SUPER_CHIP { // older interpreters incremented index registers as they worked
                        self.index_reg += self.hex_char_to_u16(chars[1]) + 1;
                    }
                },
                _ => {},
            },
            _ => {},
        } 

        if inc { 
            self.pc += 2; // increment program counter by 2
        }

        self.key_pressed = false;

        self.update_timers();
    }
    
    fn update_timers(&mut self) {
        let delta = (self.last_instant.elapsed().as_secs() * 60) as u8;
        if 255 - self.sound_timer >= delta {
            self.sound_timer -= delta;
        } else {
            self.sound_timer = 0;
        }
        if 255 - self.delay_timer >= delta {
            self.delay_timer -= delta;
        } else {
            self.delay_timer = 0;
        }
        self.last_instant = Instant::now();
    }

    fn hex_chars_to_u16(&self, chars: Vec<char>) -> u16 {
        let hex_string = chars[0..].iter().collect::<String>();
        u16::from_str_radix(&hex_string, 16).expect("Couldn't convert hex to u16")
    }

    fn hex_char_to_u16(&self, c: char) -> u16 {
        match c {
            '0' => 0,
            '1' => 1,
            '2' => 2,
            '3' => 3,
            '4' => 4,
            '5' => 5,
            '6' => 6,
            '7' => 7,
            '8' => 8,
            '9' => 9,
            'A' => 10,
            'B' => 11,
            'C' => 12,
            'D' => 13,
            'E' => 14,
            'F' => 15,
            _ => 0,
        }
    }

    fn return_from_subroutine(&mut self) { // RET
        self.pc = self.stack[self.stack.len() - 1];
        self.stack.pop().unwrap();
    }

    /// Draw the `World` state to the frame buffer.
    ///
    /// Assumes the default texture format: `wgpu::TextureFormat::Rgba8UnormSrgb`
    fn draw(&self, frame: &mut [u8]) {
        let frame_buffer = self.frame_buffer.export();
        for (i, pixel) in frame.chunks_exact_mut(4).enumerate() {
            let rgba = if frame_buffer[i] {
                [0xff, 0xff, 0xff, 0xff]
            } else {
                [0x00, 0x00, 0x00, 0xff]
            };

            pixel.copy_from_slice(&rgba);
        }
    }
}

fn main() -> Result<(), Error> {
    let rom_location = &std::env::args().collect::<Vec<String>>()[1];
    println!("Running CHIP8 ROM '{}'", rom_location);
    let data: Vec<u8> = fs::read(rom_location).unwrap();

    env_logger::init();
    let event_loop = EventLoop::new().unwrap();
    let mut input = WinitInputHelper::new();
    let window = {
        let size = LogicalSize::new((WIDTH * K)as f64, (HEIGHT * K) as f64);
        WindowBuilder::new()
            .with_title("CHIP8 Emulator")
            .with_inner_size(size)
            .with_min_inner_size(size)
            .build(&event_loop)
            .unwrap()
    };

    let mut pixels = {
        let window_size = window.inner_size();
        let surface_texture = SurfaceTexture::new(window_size.width, window_size.height, &window);
        Pixels::new(WIDTH, HEIGHT, surface_texture)?
    };
    let mut emulator = CHIP8::new(data);

    let res = event_loop.run(|event, elwt| {
        // Draw the current frame
        if let Event::WindowEvent {
            event: WindowEvent::RedrawRequested,
            ..
        } = event
        {
            emulator.draw(pixels.frame_mut());
            if let Err(err) = pixels.render() {
                log_error("pixels.render", err);
                elwt.exit();
                return;
            }
        }

        // Handle input events
        if input.update(&event) {
            // Close events
            use KeyCode::*;
            if input.key_pressed(Escape) || input.close_requested() {
                elwt.exit();
                return;
            }

            if input.key_pressed(Space) {
                emulator.pause();
            }

            /*  1 2 3 4 | 1 2 3 C
             *  Q W E R | 4 5 6 D
             *  A S D F | 7 8 9 E
             *  Z X C V | A 0 B F
             */

            let keys = vec![Digit1, Digit2, Digit3, Digit4, KeyQ, KeyW, KeyE, KeyR, KeyA, KeyS, KeyD, KeyF, KeyZ, KeyX, KeyC, KeyV];

            for key in keys {
                if input.key_pressed(key) {
                    emulator.last_key = Some(match key {
                        Digit1 => 0x1,
                        Digit2 => 0x2,
                        Digit3 => 0x3,
                        Digit4 => 0xC,
                        KeyQ   => 0x4,
                        KeyW   => 0x5,
                        KeyE   => 0x6,
                        KeyR   => 0xD,
                        KeyA   => 0x7,
                        KeyS   => 0x8,
                        KeyD   => 0x9,
                        KeyF   => 0xE,
                        KeyZ   => 0xA,
                        KeyX   => 0x0,
                        KeyC   => 0xB,
                        KeyV   => 0xF,
                        _ => 0x0, // literally impossible just pleasing my LSP
                    });
                    emulator.key_pressed = true;
                    break;
                }
            }

            // Resize the window
            if let Some(size) = input.window_resized() {
                if let Err(err) = pixels.resize_surface(size.width, size.height) {
                    log_error("pixels.resize_surface", err);
                    elwt.exit();
                    return;
                }
            }

            // Update internal state and request a redraw
            emulator.update();
            window.request_redraw();
        }
    });
    res.map_err(|e| Error::UserDefined(Box::new(e)))
}

fn log_error<E: std::error::Error + 'static>(method_name: &str, err: E) {
    error!("{method_name}() failed: {err}");
    for source in err.sources().skip(1) {
        error!("  Caused by: {source}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_conversion() {
        let data: Vec<u8> = fs::read("IBM logo.ch8").unwrap();
        let emu = CHIP8::new(data);

        assert_eq!(emu.hex_chars_to_u16(vec!['2','2','8']), 552);
    }

    #[test]
    fn most_to_least_significant_bit() {
        for i in 0..8 {
            println!("4: bit {} is {}", i, get_bit(&4u8, &i));
        }

        assert!(get_bit(&4u8, &5u8));
        assert!(!get_bit(&4u8, &4u8));
    }

    #[test]
    fn digit_splicing() {
        let number = 159;

        let digit_three = number % 10;
        let digit_two = (number % 100 - digit_three) / 10;
        let digit_one = (number - digit_two*10 - digit_three) / 100; 

        assert_eq!(digit_one, 1);
        assert_eq!(digit_two, 5);
        assert_eq!(digit_three, 9);
    }
}
