//! GPU/NPU: the right sidebar's tool for the accelerators of an AI host,
//! read over the SSH terminal's own connection: how busy each is, its
//! memory, heat and health, and the processes computing on them, which it
//! ends. NVIDIA GPUs (`nvidia-smi`) and Huawei Ascend NPUs (`npu-smi`), on
//! Linux hosts.

mod gpu_panel;
mod linux;
mod model;

pub use gpu_panel::GpuPanel;
