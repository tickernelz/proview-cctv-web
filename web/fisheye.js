(function () {
  const VERTEX = [
    '#version 300 es',
    'in vec2 a_position;',
    'out vec2 v_uv;',
    'void main() {',
    '  v_uv = a_position * 0.5 + 0.5;',
    '  gl_Position = vec4(a_position, 0.0, 1.0);',
    '}',
  ].join('\n');

  const FRAGMENT = [
    '#version 300 es',
    'precision highp float;',
    'in vec2 v_uv;',
    'out vec4 out_color;',
    'uniform sampler2D u_frame;',
    'uniform vec2 u_imageSize;',
    'uniform vec2 u_center;',
    'uniform float u_scale;',
    'uniform float u_maxTheta;',
    'uniform int u_mode;',
    'uniform float u_halfExtent;',
    'uniform float u_aspect;',
    'uniform float u_pan;',
    'uniform float u_tilt;',
    'uniform float u_camZ;',
    'uniform float u_mountPitch;',
    'uniform int u_lutCount;',
    'uniform float u_lut[65];',
    'const float PI = 3.141592653589793;',
    'const float TAU = 6.283185307179586;',
    'vec3 rotateView(vec3 value) {',
    '  float cosPan = cos(u_pan);',
    '  float sinPan = sin(u_pan);',
    '  float cosTilt = cos(u_tilt);',
    '  float sinTilt = sin(u_tilt);',
    '  vec3 panned = vec3(value.x * cosPan + value.z * sinPan, value.y, -value.x * sinPan + value.z * cosPan);',
    '  return vec3(panned.x, panned.y * cosTilt - panned.z * sinTilt, panned.y * sinTilt + panned.z * cosTilt);',
    '}',
    'vec3 viewRay(vec2 point) {',
    '  float squared = dot(point, point);',
    '  float inside = sqrt(max(1.0 + squared * (1.0 - u_camZ), 0.0));',
    '  float z = (squared * u_camZ - inside) / (squared + 1.0);',
    '  float scale = u_camZ - z;',
    '  return normalize(vec3(scale * point.x, scale * point.y, -z));',
    '}',
    'vec3 pitchDir(vec3 value) {',
    '  float c = cos(u_mountPitch);',
    '  float s = sin(u_mountPitch);',
    '  return vec3(value.x, value.y * c - value.z * s, value.y * s + value.z * c);',
    '}',
    'vec3 equirectRay(vec2 uv) {',
    '  float bearing = (0.25 - uv.x) * TAU;',
    '  float elevation = (0.5 - uv.y) * PI;',
    '  return vec3(cos(elevation) * cos(bearing), cos(elevation) * sin(bearing), sin(elevation));',
    '}',
    'void main() {',
    '  vec3 direction;',
    '  if (u_mode == 0) {',
    '    direction = pitchDir(equirectRay(v_uv));',
    '  } else {',
    '    vec2 point = vec2((v_uv.x * 2.0 - 1.0) * u_halfExtent * u_aspect, (0.5 - v_uv.y) * 2.0 * u_halfExtent);',
    '    direction = rotateView(viewRay(point));',
    '  }',
    '  float theta = acos(clamp(direction.z, -1.0, 1.0));',
    '  if (theta > u_maxTheta) {',
    '    out_color = vec4(0.0);',
    '    return;',
    '  }',
    '  float radiusPx = theta * u_scale;',
    '  if (u_lutCount > 1) {',
    '    int index = int(floor(theta / u_maxTheta * float(u_lutCount - 1)));',
    '    index = clamp(index, 0, u_lutCount - 2);',
    '    float lower = u_lut[index];',
    '    float upper = u_lut[index + 1];',
    '    float span = theta / u_maxTheta * float(u_lutCount - 1) - float(index);',
    '    radiusPx = mix(lower, upper, clamp(span, 0.0, 1.0));',
    '  }',
    '  float azimuth = atan(direction.y, direction.x);',
    '  vec2 source = u_center + vec2(cos(azimuth), sin(azimuth)) * radiusPx;',
    '  vec2 uv = source / u_imageSize;',
    '  if (uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0) {',
    '    out_color = vec4(0.0);',
    '    return;',
    '  }',
    '  out_color = texture(u_frame, uv);',
    '}',
  ].join('\n');

  function compile(gl, type, source) {
    const shader = gl.createShader(type);
    gl.shaderSource(shader, source);
    gl.compileShader(shader);
    if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) {
      const log = gl.getShaderInfoLog(shader);
      gl.deleteShader(shader);
      throw new Error('shader compile failed: ' + log);
    }
    return shader;
  }

  function radians(degrees) {
    return (degrees * Math.PI) / 180;
  }

  class FisheyeDewarp {
    constructor(canvas) {
      this.canvas = canvas;
      this.gl = canvas.getContext('webgl2', { premultipliedAlpha: false, alpha: true });
      if (!this.gl) {
        throw new Error('webgl2 unavailable');
      }
      const gl = this.gl;
      const program = gl.createProgram();
      gl.attachShader(program, compile(gl, gl.VERTEX_SHADER, VERTEX));
      gl.attachShader(program, compile(gl, gl.FRAGMENT_SHADER, FRAGMENT));
      gl.linkProgram(program);
      if (!gl.getProgramParameter(program, gl.LINK_STATUS)) {
        throw new Error('program link failed: ' + gl.getProgramInfoLog(program));
      }
      this.program = program;
      this.buffer = gl.createBuffer();
      gl.bindBuffer(gl.ARRAY_BUFFER, this.buffer);
      gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1, -1, 1, -1, -1, 1, 1, 1]), gl.STATIC_DRAW);
      const position = gl.getAttribLocation(program, 'a_position');
      gl.enableVertexAttribArray(position);
      gl.vertexAttribPointer(position, 2, gl.FLOAT, false, 0, 0);
      this.texture = gl.createTexture();
      gl.bindTexture(gl.TEXTURE_2D, this.texture);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
      this.frame = null;
      this.radius = 240;
      this.centerX = 256;
      this.centerY = 256;
      this.imageWidth = 512;
      this.imageHeight = 512;
      this.maxTheta = Math.PI / 2;
      this.scale = this.radius / this.maxTheta;
      this.mode = 0;
      this.pan = 0;
      this.tilt = 0;
      this.fov = radians(90);
      this.camZ = 0;
      this.mountPitch = 0;
      this.lut = null;
      this.uniforms = {};
      for (const name of [
        'u_frame',
        'u_imageSize',
        'u_center',
        'u_scale',
        'u_maxTheta',
        'u_mode',
        'u_halfExtent',
        'u_aspect',
        'u_pan',
        'u_tilt',
        'u_camZ',
        'u_mountPitch',
        'u_lutCount',
        'u_lut',
      ]) {
        this.uniforms[name] = gl.getUniformLocation(program, name);
      }
    }

    setLens(options) {
      if (options.imageWidth) this.imageWidth = options.imageWidth;
      if (options.imageHeight) this.imageHeight = options.imageHeight;
      if (options.centerX !== undefined) this.centerX = options.centerX;
      if (options.centerY !== undefined) this.centerY = options.centerY;
      if (options.radius !== undefined) this.radius = options.radius;
      if (options.maxThetaDeg !== undefined) this.maxTheta = radians(options.maxThetaDeg);
      this.lut = FisheyeDewarp.resampleLut(options.lut);
      this.scale = this.radius / this.maxTheta;
    }

    setView(options) {
      if (options.mode !== undefined) this.mode = options.mode === 'ptz' ? 1 : 0;
      if (options.panDeg !== undefined) this.pan = radians(options.panDeg);
      if (options.tiltDeg !== undefined) this.tilt = radians(options.tiltDeg);
      if (options.fovDeg !== undefined) this.fov = radians(Math.min(179, Math.max(1, options.fovDeg)));
      if (options.camZ !== undefined) this.camZ = options.camZ;
    }

    static resampleLut(values) {
      if (!Array.isArray(values) || values.length < 2) {
        return null;
      }
      const width = 65;
      const out = new Array(width);
      for (let slot = 0; slot < width; slot += 1) {
        const position = (slot / (width - 1)) * (values.length - 1);
        const index = Math.min(values.length - 2, Math.floor(position));
        const fraction = Math.min(1, position - index);
        out[slot] = values[index] + (values[index + 1] - values[index]) * fraction;
      }
      return out;
    }

    setMount(name) {
      const pitches = { ceiling: 0, table: 180, wall: 90 };
      const pitch = pitches[String(name).toLowerCase()];
      this.mountPitch = ((pitch === undefined ? 0 : pitch) * Math.PI) / 180;
    }

    uploadFrame(source) {
      const gl = this.gl;
      gl.bindTexture(gl.TEXTURE_2D, this.texture);
      gl.pixelStorei(gl.UNPACK_FLIP_Y_WEBGL, false);
      gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, source);
      this.frame = source;
    }

    render() {
      if (!this.frame) {
        return false;
      }
      const gl = this.gl;
      const uniforms = this.uniforms;
      gl.viewport(0, 0, this.canvas.width, this.canvas.height);
      gl.useProgram(this.program);
      gl.bindBuffer(gl.ARRAY_BUFFER, this.buffer);
      const position = gl.getAttribLocation(this.program, 'a_position');
      gl.enableVertexAttribArray(position);
      gl.vertexAttribPointer(position, 2, gl.FLOAT, false, 0, 0);
      gl.activeTexture(gl.TEXTURE0);
      gl.bindTexture(gl.TEXTURE_2D, this.texture);
      gl.uniform1i(uniforms.u_frame, 0);
      gl.uniform2f(uniforms.u_imageSize, this.imageWidth, this.imageHeight);
      gl.uniform2f(uniforms.u_center, this.centerX, this.centerY);
      gl.uniform1f(uniforms.u_scale, this.scale);
      gl.uniform1f(uniforms.u_maxTheta, this.maxTheta);
      gl.uniform1i(uniforms.u_mode, this.mode);
      gl.uniform1f(uniforms.u_halfExtent, Math.tan(this.fov * 0.5));
      gl.uniform1f(uniforms.u_aspect, this.canvas.width / Math.max(1, this.canvas.height));
      gl.uniform1f(uniforms.u_pan, this.pan);
      gl.uniform1f(uniforms.u_tilt, this.tilt);
      gl.uniform1f(uniforms.u_camZ, this.camZ);
      gl.uniform1f(uniforms.u_mountPitch, this.mountPitch);
      const lut = new Float32Array(65);
      if (this.lut) {
        for (let index = 0; index < Math.min(65, this.lut.length); index += 1) {
          lut[index] = this.lut[index] * this.radius;
        }
      }
      gl.uniform1i(uniforms.u_lutCount, this.lut ? Math.min(65, this.lut.length) : 0);
      gl.uniform1fv(uniforms.u_lut, lut);
      gl.clearColor(0, 0, 0, 0);
      gl.clear(gl.COLOR_BUFFER_BIT);
      gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
      return true;
    }
  }

  function overlayCanvas(video) {
    const canvas = document.createElement('canvas');
    canvas.style.cssText =
      'position:absolute;inset:0;width:100%;height:100%;pointer-events:none;';
    const parent = video.parentElement;
    if (parent) {
      if (window.getComputedStyle(parent).position === 'static') {
        parent.style.position = 'relative';
      }
      parent.appendChild(canvas);
    }
    return canvas;
  }

  function lensFromVideo(video) {
    const width = video.videoWidth;
    const height = video.videoHeight;
    const radius = Math.min(width, height) * 0.469;
    return {
      imageWidth: width,
      imageHeight: height,
      centerX: width / 2,
      centerY: height / 2,
      radius,
      maxThetaDeg: 90,
    };
  }

  const MODEL = {
    library: 'libfisheye.so',
    projection: 'equidistant sphere with the vendor bearing/elevation equirect convention',
    radiusToTheta: 'theta = radiusPx / (radius / maxTheta)',
    viewRay: 'z = (a*camZ - sqrt(1 + a*(1 - camZ))) / (a + 1), x = (camZ - z) * px, y = (camZ - z) * py',
    functions: [
      'CFisheyeMap::GetThetaXYFromFlatXY',
      'Fisheye_GenAndSetLensInfo',
      'Fisheye_PanTiltToInVPicturePoint',
      'FisheyeGL_UpdatePanoramaShaderParameters',
    ],
  };

  function attachLive(options) {
    const settings = options || {};
    const video = settings.video || document.getElementById('video');
    if (!video) {
      return null;
    }
    const canvas = settings.canvas || overlayCanvas(video);
    const dewarp = new FisheyeDewarp(canvas);
    dewarp.setMount(settings.mount || 'ceiling');
    dewarp.setView(settings.view || { mode: 'equirect' });
    dewarp.setLens(settings.lens || lensFromVideo(video));
    let running = true;
    const tick = () => {
      if (!running) {
        return;
      }
      const current = document.getElementById(video.id) || video;
      if (!canvas.isConnected && current.parentElement) {
        if (window.getComputedStyle(current.parentElement).position === 'static') {
          current.parentElement.style.position = 'relative';
        }
        current.parentElement.appendChild(canvas);
      }
      const width = canvas.clientWidth || canvas.width;
      const height = canvas.clientHeight || canvas.height;
      if (canvas.width !== width || canvas.height !== height) {
        canvas.width = width;
        canvas.height = height;
      }
      if (current.readyState >= 2 && current.videoWidth > 0) {
        if (dewarp.imageWidth !== current.videoWidth) {
          const lens = lensFromVideo(current);
          dewarp.setLens(lens);
          if (!settings.lens) {
            dewarp.setMount(settings.mount || 'ceiling');
            dewarp.setView(settings.view || { mode: 'equirect' });
          }
        }
        dewarp.uploadFrame(current);
        dewarp.render();
      }
      window.requestAnimationFrame(tick);
    };
    window.requestAnimationFrame(tick);
    return {
      dewarp,
      canvas,
      stop() {
        running = false;
        if (canvas.parentElement) {
          canvas.parentElement.removeChild(canvas);
        }
      },
    };
  }

  function autoAttach() {
    const params = new URLSearchParams(window.location.search);
    const mode = params.get('fisheye');
    if (!mode) {
      return null;
    }
    window.ProviewFisheyeLive = attachLive({
      mount: params.get('mount') || 'ceiling',
      view: {
        mode: mode === 'ptz' ? 'ptz' : 'equirect',
        panDeg: Number(params.get('pan') || 0),
        tiltDeg: Number(params.get('tilt') || 0),
        fovDeg: Number(params.get('fov') || 90),
      },
    });
    return window.ProviewFisheyeLive;
  }

  window.ProviewFisheye = {
    create(canvas) {
      return new FisheyeDewarp(canvas);
    },
    attachLive,
    autoAttach,
    supported() {
      const probe = document.createElement('canvas');
      return typeof probe.getContext === 'function' && probe.getContext('webgl2') !== null;
    },
    model: MODEL,
  };

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', autoAttach);
  } else {
    autoAttach();
  }
})();
