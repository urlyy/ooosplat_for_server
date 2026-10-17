#!/usr/bin/env python3
"""Render a deterministic textured cube orbit for pipeline smoke testing.
Test-only dependencies: numpy, opencv (Ubuntu python3-numpy/python3-opencv), ffmpeg.
This synthetic scene tests execution, not reconstruction quality on real footage.
"""
import argparse
from pathlib import Path
import subprocess
import cv2
import numpy as np


def texture(seed, size=768):
    rng = np.random.default_rng(seed)
    image = rng.integers(80, 210, (size, size, 3), dtype=np.uint8)
    image = cv2.GaussianBlur(image, (5, 5), 0)
    for index in range(700):
        x, y = rng.integers(12, size - 12, 2)
        color = tuple(int(c) for c in rng.integers(5, 250, 3))
        radius = int(rng.integers(3, 14))
        cv2.circle(image, (int(x), int(y)), radius, color, -1, cv2.LINE_AA)
        if index % 8 == 0:
            cv2.putText(image, str(index), (int(x), int(y)), cv2.FONT_HERSHEY_SIMPLEX,
                        .45, (20, 20, 20), 1, cv2.LINE_AA)
    return image


def render(output, frames=180):
    width, height, fps = 960, 720, 30
    corners = np.array([[-1,-1,0], [1,-1,0], [1,1,0], [-1,1,0],
                        [-1,-1,2], [1,-1,2], [1,1,2], [-1,1,2]], dtype=float)
    faces = [[0,1,5,4], [1,2,6,5], [2,3,7,6], [3,0,4,7], [4,5,6,7]]
    textures = [texture(seed) for seed in range(6)]
    floor = np.array([[-5,-5,-.02], [5,-5,-.02], [5,5,-.02], [-5,5,-.02]])
    source = np.array([[0,0], [767,0], [767,767], [0,767]], dtype=np.float32)
    output.parent.mkdir(parents=True, exist_ok=True)
    command = ['ffmpeg', '-hide_banner', '-loglevel', 'error', '-y', '-f', 'rawvideo',
               '-pixel_format', 'bgr24', '-video_size', f'{width}x{height}', '-framerate', str(fps),
               '-i', '-', '-c:v', 'libx264', '-crf', '18', '-pix_fmt', 'yuv420p', str(output)]
    process = subprocess.Popen(command, stdin=subprocess.PIPE)
    try:
        for index in range(frames):
            angle = np.deg2rad(20 + 100 * index / (frames - 1))
            camera = np.array([5.5*np.sin(angle), 5.5*np.cos(angle), 3.6])
            forward = np.array([0., 0., .8]) - camera
            forward /= np.linalg.norm(forward)
            right = np.cross(forward, [0., 0., 1.]); right /= np.linalg.norm(right)
            down = np.cross(forward, right)
            rotation = np.stack([right, down, forward])
            def project(points):
                local = (points - camera) @ rotation.T
                return (local[:, :2] / local[:, 2:] * 720 + [width/2, height/2]).astype(np.float32)
            image = np.full((height, width, 3), 160, np.uint8)
            ordered = sorted(enumerate(faces), key=lambda item: np.linalg.norm(corners[item[1]].mean(axis=0)-camera), reverse=True)
            for face_index, points in [(-1, floor)] + [(i, corners[f]) for i, f in ordered]:
                destination = project(points)
                transform = cv2.getPerspectiveTransform(source, destination)
                warped = cv2.warpPerspective(textures[face_index], transform, (width, height))
                mask = np.zeros((height, width), np.uint8)
                cv2.fillConvexPoly(mask, np.rint(destination).astype(np.int32), 255)
                image[mask != 0] = warped[mask != 0]
            process.stdin.write(image.tobytes())
    finally:
        process.stdin.close()
    if process.wait() != 0:
        raise RuntimeError('ffmpeg failed to encode the test video')
    print(output)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    render(args.output)
