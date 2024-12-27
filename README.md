<!--lint disable no-literal-urls-->
<div align="center">
  <h1>Rust AV1 Decoder</h1>
</div>
<br/>
<div align="center">
  <strong>A AV1 decoder implemented in pure rust.</strong>
</div>
<div align="center">
  <img src="https://img.shields.io/github/license/mycrl/toy-rav1d"/>
  <img src="https://img.shields.io/github/issues/mycrl/toy-rav1d"/>
  <img src="https://img.shields.io/github/stars/mycrl/toy-rav1d"/>
</div>
<div align="center">
  <sup>This is an experimental project currently in progress.</sup>
</div>

---

Unlike existing projects, this is an AV1 decoder implemented entirely from scratch in Rust. Note, however, that this is an experimental project, not intended for production use, so there is no particular focus on performance, and only basic features will be implemented.

This project is mainly for learning and understanding the features of AV1 encoding and video encoding.

At work I'm developing the Obu parser part of an AV1 decoder, but the project I'm involved in at work uses a lot of ASM and C code, which has given me some interest in implementing an AV1 decoder in its entirety using Rust. This is a challenge for me because the core modules (macroblock reconstruction and the compression algorithm part, etc.) are not something I'm involved in.
