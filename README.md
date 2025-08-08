
# 🎯 tbf – Twitch Broadcast Finder

> A powerful command-line tool to effortlessly find and manage Twitch VOD playlists and clips.

![Showcase](https://raw.githubusercontent.com/vyneer/tbf/master/showcase.gif)

[![Build Status](https://img.shields.io/badge/build-passing-brightgreen.svg)](https://github.com/vyneer/tbf)
[![MIT License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![GitHub release](https://img.shields.io/github/v/release/vyneer/tbf)](https://github.com/vyneer/tbf/releases)

---

## 📖 Table of Contents

* [About](#-about-the-project)
* [Getting Started](#-getting-started)
* [Usage](#-usage)
* [Features](#-features)
* [Roadmap](#-roadmap)
* [Contributing](#-contributing)
* [License](#-license)
* [Contact](#-contact)

---

## 🌟 About The Project

`tbf` is a command-line interface (CLI) tool designed to simplify the process of finding and working with Twitch VODs (Video on Demand) and clips. Whether you need to generate a direct `m3u8` playlist URL, search for a VOD within a specific timeframe, or extract clips, `tbf` provides a set of powerful and easy-to-use commands to get the job done.

This tool is perfect for researchers, archivists, and anyone who needs programmatic access to Twitch's video content.

---

## 🚀 Getting Started

### Prerequisites

Before you begin, ensure you have the following installed:

*   **Rust**: `tbf` is built with Rust. If you don't have it installed, you can get it from [rust-lang.org](https://www.rust-lang.org/tools/install).

### Installation

You can install `tbf` in one of two ways:

#### From Source

If you have Rust and Cargo installed, you can build and install `tbf` directly from the source:
```bash
cargo install --git https://github.com/vyneer/tbf
```

#### From Releases

Alternatively, you can download a pre-compiled binary for your operating system from the [Releases Page](https://github.com/vyneer/tbf/releases).

---

## 🛠️ Usage

`tbf` offers several subcommands to perform different actions. Here are some of the most common use cases:

### Interactive Mode

If you're not sure where to start, you can run `tbf` without any arguments to enter an interactive mode that will guide you through the available options.
```bash
tbf
```

### `exact`

Generate and verify a direct `m3u8` URL for a VOD with a known timestamp.
```bash
tbf exact [FLAGS] <username> <id> <timestamp>
```
**Example:**
```bash
tbf exact destiny 39700667438 1605781794
```

### `bruteforce`

Search for a VOD within a given range of timestamps. This is useful when you don't know the exact timestamp of the broadcast.
```bash
tbf bruteforce [FLAGS] <username> <id> <from> <to>
```
**Example:**
```bash
tbf bruteforce destiny 39700667438 1605781694 1605781894
```

### `clipforce`

Scan a VOD to discover all available clips within a specified time range.
```bash
tbf clipforce [FLAGS] <id> <start> <end>
```
**Example:**
```bash
tbf clipforce 39700667438 0 3600
```

---

## ✨ Features

*   **Interactive Mode**: A user-friendly interface to guide you through the process.
*   **Direct URL Generation**: Quickly get a direct `.m3u8` VOD URL.
*   **Timestamp Bruteforcing**: Find VODs even without knowing the exact start time.
*   **Clip Discovery**: Easily find and extract clips from a VOD.
*   **Multiple Sources**: Supports fetching data from TwitchTracker and StreamsCharts.

---

## 🗺️ Roadmap

See the [open issues](https://github.com/vyneer/tbf/issues) for a list of proposed features (and known issues).

---

## 🤝 Contributing

Contributions are what make the open-source community such an amazing place to learn, inspire, and create. Any contributions you make are **greatly appreciated**.

If you have a suggestion that would make this better, please fork the repo and create a pull request. You can also simply open an issue with the tag "enhancement".

1.  Fork the Project
2.  Create your Feature Branch (`git checkout -b feature/AmazingFeature`)
3.  Commit your Changes (`git commit -m 'Add some AmazingFeature'`)
4.  Push to the Branch (`git push origin feature/AmazingFeature`)
5.  Open a Pull Request

Don't forget to give the project a star! Thanks again!

### Code of Conduct

Please note that this project is released with a [Contributor Code of Conduct](https://www.contributor-covenant.org/version/2/1/code_of_conduct/). By participating in this project you agree to abide by its terms.

---

## 📄 License

Distributed under the MIT License. See `LICENSE` for more information.

---

## 📞 Contact

Vyneer - [@vyneer](https://twitter.com/vyneer)

Project Link: [https://github.com/vyneer/tbf](https://github.com/vyneer/tbf)